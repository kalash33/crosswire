//! Writer thread, bounded queue, journal spill, events and audit log.

use bytes::Bytes;
use crosswire_core::Span;
use crosswire_model::{
    Body, ConnectionInfo, Event, Exchange, Fidelity, FramingRecord, Head, HeadSpans,
    HeaderSpanRecord, Marker, Message, MessageKind, SinkError, StartLineSpans,
    HTTP1_PARSER_VERSION,
};
use crosswire_store::{BodySide, BodyStore, EventStream, Project, Store, StoreConfig};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use ulid::Ulid;

/// Deterministic, poorly compressible bytes.
fn noise(n: usize, seed: u64) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

fn msg(body: &[u8]) -> Message {
    Message::new(
        MessageKind::Normalized,
        Fidelity::Normalized,
        Head::Normalized { map: Vec::new() },
        FramingRecord::ContentLength {
            len: body.len() as u64,
        },
        Body::Inline {
            bytes: Bytes::copy_from_slice(body),
        },
        ConnectionInfo::new("example.test", 443),
    )
}

fn exchange(req: &[u8], resp: &[u8]) -> Exchange {
    let mut ex = Exchange::new(msg(req));
    ex.set_response(msg(resp));
    ex
}

fn cfg() -> StoreConfig {
    StoreConfig {
        batch_interval: Duration::from_millis(20),
        ..StoreConfig::default()
    }
}

fn new_store(tmp: &tempfile::TempDir, cfg: StoreConfig) -> (PathBuf, Store) {
    let dir = tmp.path().join("p");
    let store = Store::open(Project::create(&dir).unwrap(), cfg).unwrap();
    (dir, store)
}

/// Polls `ev` until `want` are all committed or `limit` passes; returns the committed set.
fn wait_committed(ev: &mut EventStream, want: &[Ulid], limit: Duration) -> BTreeSet<Ulid> {
    let deadline = Instant::now() + limit;
    let mut got = BTreeSet::new();
    while Instant::now() < deadline {
        match ev.try_recv() {
            Some(Event::Committed { ids, .. }) => {
                got.extend(ids);
                if want.iter().all(|w| got.contains(w)) {
                    break;
                }
            }
            Some(_) => {}
            None => std::thread::sleep(Duration::from_millis(2)),
        }
    }
    got
}

fn journal_files(dir: &Path) -> usize {
    std::fs::read_dir(dir.join("journal"))
        .unwrap()
        .filter(|e| {
            let e = e.as_ref().unwrap();
            e.file_type().unwrap().is_file() && e.file_name().to_string_lossy().ends_with(".json")
        })
        .count()
}

/// A gap marker row: `from_ulid`, `to_ulid`, `detail`.
struct Gap {
    from: Option<Ulid>,
    to: Ulid,
    detail: String,
}

fn gap_markers(dir: &Path) -> Vec<Gap> {
    // Read without taking the project lock, which would rewrite its status.
    let conn = rusqlite::Connection::open_with_flags(
        dir.join("project.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let mut st = conn
        .prepare("SELECT from_ulid, to_ulid, detail FROM markers WHERE kind = 'gap' ORDER BY ulid")
        .unwrap();
    let rows = st
        .query_map([], |r| {
            Ok((
                r.get::<_, Option<String>>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .unwrap()
        .map(|r| {
            let (from, to, detail) = r.unwrap();
            Gap {
                from: from.map(|f| Ulid::from_string(&f).unwrap()),
                to: Ulid::from_string(&to).unwrap(),
                detail,
            }
        })
        .collect();
    rows
}

#[tokio::test]
async fn committed_follows_received_and_is_readable() {
    let tmp = tempfile::tempdir().unwrap();
    let (dir, store) = new_store(&tmp, cfg());
    let mut ev = store.subscribe();
    let sink = store.sink();
    let big = noise(20 * 1024, 7);
    let exs = [
        exchange(b"a", b"small response"),
        exchange(&big, b""),
        exchange(b"c", &noise(40 * 1024, 8)),
    ];
    let ids: Vec<Ulid> = exs.iter().map(Exchange::id).collect();
    for ex in exs.clone() {
        sink.submit(ex).unwrap();
    }
    let mut received = BTreeSet::new();
    let mut committed = BTreeSet::new();
    let mut last_seq = 0;
    tokio::time::timeout(Duration::from_secs(10), async {
        while !ids.iter().all(|i| committed.contains(i)) {
            let e = ev.recv().await.expect("stream open");
            assert!(e.seq() > last_seq, "seq must increase");
            last_seq = e.seq();
            match e {
                Event::Received { id, .. } => {
                    received.insert(id);
                }
                Event::Committed { ids, .. } => {
                    for id in ids {
                        assert!(received.contains(&id), "committed before received");
                        committed.insert(id);
                    }
                }
                other => panic!("unexpected {other:?}"),
            }
        }
    })
    .await
    .expect("all committed within 10 s");

    let conn = store.reader().unwrap();
    for ex in &exs {
        let key = ex.id().to_string();
        let (state, json): (String, String) = conn
            .query_row(
                "SELECT state, json FROM exchanges WHERE ulid = ?1",
                [&key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(state, "committed");
        let stored: Exchange = serde_json::from_str(&json).unwrap();
        assert!(
            matches!(stored.request().body(), Body::Blob { .. }),
            "bodies are not kept in the json"
        );
        let Body::Inline { bytes } = ex.request().body() else {
            unreachable!()
        };
        let got = BodyStore::get(&conn, &dir, ex.id(), BodySide::Req).unwrap();
        assert_eq!(got.as_deref(), Some(&bytes[..]));
        let Body::Inline { bytes } = ex.response().unwrap().body() else {
            unreachable!()
        };
        let got = BodyStore::get(&conn, &dir, ex.id(), BodySide::Resp).unwrap();
        assert_eq!(got.as_deref(), Some(&bytes[..]));
    }
    let last: String = conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'last_committed_ulid'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(last, ids.iter().max().unwrap().to_string());
    store.close().unwrap();
}

#[test]
fn batch_boundaries() {
    let tmp = tempfile::tempdir().unwrap();
    let (_dir, store) = new_store(
        &tmp,
        StoreConfig {
            batch_max: 2,
            batch_interval: Duration::from_secs(10),
            ..StoreConfig::default()
        },
    );
    let mut ev = store.subscribe();
    let sink = store.sink();
    let (a, b) = (exchange(b"a", b"a"), exchange(b"b", b"b"));
    let want = [a.id(), b.id()];
    sink.submit(a).unwrap();
    sink.submit(b).unwrap();
    let got = wait_committed(&mut ev, &want, Duration::from_secs(2));
    assert!(
        want.iter().all(|w| got.contains(w)),
        "a full batch commits at once"
    );

    let c = exchange(b"c", b"c");
    let cid = c.id();
    sink.submit(c).unwrap();
    let got = wait_committed(&mut ev, &[cid], Duration::from_secs(1));
    assert!(
        !got.contains(&cid),
        "a partial batch waits for the interval"
    );
    store.close().unwrap();

    let tmp = tempfile::tempdir().unwrap();
    let (_dir, store) = new_store(
        &tmp,
        StoreConfig {
            batch_max: 2,
            batch_interval: Duration::from_millis(50),
            ..StoreConfig::default()
        },
    );
    let mut ev = store.subscribe();
    let d = exchange(b"d", b"d");
    let did = d.id();
    let start = Instant::now();
    store.sink().submit(d).unwrap();
    let got = wait_committed(&mut ev, &[did], Duration::from_millis(200));
    assert!(
        got.contains(&did),
        "committed within 200 ms (took {:?})",
        start.elapsed()
    );
    store.close().unwrap();
}

#[test]
fn queue_full_spills_to_journal_and_drains() {
    let tmp = tempfile::tempdir().unwrap();
    let pause = Arc::new(Mutex::new(()));
    let (dir, store) = new_store(
        &tmp,
        StoreConfig {
            queue_max: 2,
            test_pause: Some(Arc::clone(&pause)),
            ..cfg()
        },
    );
    let mut ev = store.subscribe();
    let sink = store.sink();
    let guard = pause.lock().unwrap();
    let exs: Vec<Exchange> = (0..10u64)
        .map(|i| exchange(&noise(100, i), &noise(30 * 1024, i + 100)))
        .collect();
    let ids: Vec<Ulid> = exs.iter().map(Exchange::id).collect();
    for ex in exs {
        sink.submit(ex).unwrap();
    }
    assert_eq!(journal_files(&dir), 8, "two queued, eight spilled");
    drop(guard);
    let got = wait_committed(&mut ev, &ids, Duration::from_secs(10));
    assert!(ids.iter().all(|i| got.contains(i)), "all ten committed");
    assert_eq!(journal_files(&dir), 0, "journal drained");
    let conn = store.reader().unwrap();
    let n: i64 = conn
        .query_row("SELECT count(*) FROM exchanges", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 10);
    store.close().unwrap();
}

#[test]
fn queue_byte_budget_spills() {
    let tmp = tempfile::tempdir().unwrap();
    let pause = Arc::new(Mutex::new(()));
    let (dir, store) = new_store(
        &tmp,
        StoreConfig {
            queue_bytes_max: 100 * 1024,
            test_pause: Some(Arc::clone(&pause)),
            ..cfg()
        },
    );
    let mut ev = store.subscribe();
    let guard = pause.lock().unwrap();
    // Each is about 41 KiB of messages plus 1 KiB: two fit in 100 KiB.
    let exs: Vec<Exchange> = (0..5u64)
        .map(|i| exchange(&noise(20 * 1024, i), &noise(20 * 1024, i + 50)))
        .collect();
    let ids: Vec<Ulid> = exs.iter().map(Exchange::id).collect();
    for ex in exs {
        store.sink().submit(ex).unwrap();
    }
    assert_eq!(journal_files(&dir), 3);
    drop(guard);
    let got = wait_committed(&mut ev, &ids, Duration::from_secs(10));
    assert!(ids.iter().all(|i| got.contains(i)));
    store.close().unwrap();
}

#[test]
fn lagged_subscriber_gets_lagged_event() {
    let tmp = tempfile::tempdir().unwrap();
    let (_dir, store) = new_store(
        &tmp,
        StoreConfig {
            event_capacity: 4,
            ..cfg()
        },
    );
    let mut ev = store.subscribe();
    let sink = store.sink();
    for i in 0..20u8 {
        sink.submit(exchange(&[i], &[i])).unwrap();
    }
    let first = ev.try_recv().expect("an event");
    let Event::Lagged { missed, seq } = first else {
        panic!("expected Lagged, got {first:?}");
    };
    assert!(missed > 0);
    let next = ev.try_recv().expect("the oldest retained event");
    assert_eq!(
        next.seq(),
        seq + 1,
        "lagged accounts for every seq before the next event"
    );
    store.close().unwrap();
}

#[test]
fn audit_rows_are_fsynced_json_lines() {
    let tmp = tempfile::tempdir().unwrap();
    let (dir, store) = new_store(&tmp, cfg());
    store
        .audit("cli", "scope.change", "example.test", "ok")
        .unwrap();
    store
        .audit("ui", "repeater.send", "tab \"1\"\nx", "denied")
        .unwrap();
    let text = std::fs::read_to_string(dir.join("audit.jsonl")).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2);
    let second: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
    for key in ["ts", "ulid", "actor", "action", "target", "result"] {
        assert!(second.get(key).is_some(), "missing {key}");
    }
    assert_eq!(second["target"], "tab \"1\"\nx");
    assert_eq!(second["actor"], "ui");
    assert!(Ulid::from_string(second["ulid"].as_str().unwrap()).is_ok());
    store.close().unwrap();
}

#[test]
fn audit_rotates_one_generation() {
    let tmp = tempfile::tempdir().unwrap();
    let (dir, store) = new_store(
        &tmp,
        StoreConfig {
            audit_rotate_bytes: 400,
            ..cfg()
        },
    );
    for i in 0..10 {
        store
            .audit("cli", "export", &format!("t{i}"), "ok")
            .unwrap();
    }
    let cur = std::fs::read_to_string(dir.join("audit.jsonl")).unwrap();
    let old = std::fs::read_to_string(dir.join("audit.jsonl.1")).unwrap();
    assert!(cur.len() <= 400 && old.len() <= 400);
    assert!(cur.lines().last().unwrap().contains("\"t9\""));
    for l in cur.lines().chain(old.lines()) {
        serde_json::from_str::<serde_json::Value>(l).unwrap();
    }
    assert!(!dir.join("audit.jsonl.2").exists());
    store.close().unwrap();
}

#[test]
fn unclean_shutdown_writes_gap_marker_on_reopen() {
    let tmp = tempfile::tempdir().unwrap();
    let (dir, store) = new_store(&tmp, cfg());
    let mut ev = store.subscribe();
    let ex = exchange(b"x", b"y");
    let id = ex.id();
    store.sink().submit(ex).unwrap();
    assert!(wait_committed(&mut ev, &[id], Duration::from_secs(5)).contains(&id));
    // Dropping without close stops the writer but must not record a clean shutdown.
    drop(store);
    let lock = std::fs::read_to_string(dir.join("project.lock")).unwrap();
    assert!(lock.contains("clean=0"));
    assert!(gap_markers(&dir).is_empty());

    let store = Store::open(Project::open(&dir).unwrap(), cfg()).unwrap();
    store.close().unwrap();
    let gaps = gap_markers(&dir);
    assert_eq!(gaps.len(), 1);
    // The window runs from the last commit (with slack) to the restart, and
    // brackets everything submitted after that commit.
    let from = gaps[0].from.expect("a watermark was recorded");
    assert!(
        from <= id,
        "the window starts at or before the committed exchange"
    );
    assert!(gaps[0].to > id && gaps[0].to > from);
    assert_eq!(gaps[0].detail, "unclean shutdown");

    // A clean close leaves clean=1 and the next open adds no marker.
    let lock = std::fs::read_to_string(dir.join("project.lock")).unwrap();
    assert!(lock.contains("clean=1"));
    let store = Store::open(Project::open(&dir).unwrap(), cfg()).unwrap();
    store.close().unwrap();
    assert_eq!(gap_markers(&dir).len(), 1);
}

#[test]
fn clean_flag_is_reset_on_open() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("p");
    let p = Project::create(&dir).unwrap();
    p.mark_clean().unwrap();
    let store = Store::open(p, cfg()).unwrap();
    let lock = std::fs::read_to_string(dir.join("project.lock")).unwrap();
    assert!(
        lock.contains("clean=0"),
        "an open store is never marked clean"
    );
    drop(store);
}

#[test]
fn journal_is_replayed_on_open_idempotently() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("p");
    let p = Project::create(&dir).unwrap();
    let a = exchange(b"a", &noise(20 * 1024, 1));
    let b = exchange(b"b", b"b");
    for ex in [&a, &b] {
        std::fs::write(
            dir.join("journal").join(format!("{}.json", ex.id())),
            serde_json::to_vec(ex).unwrap(),
        )
        .unwrap();
    }
    // A half-written spill (never acknowledged) is discarded.
    std::fs::write(dir.join("journal").join("x.json.tmp"), b"{").unwrap();
    let store = Store::open(p, cfg()).unwrap();
    assert_eq!(journal_files(&dir), 0);
    assert!(!dir.join("journal").join("x.json.tmp").exists());
    let conn = store.reader().unwrap();
    for ex in [&a, &b] {
        let state: String = conn
            .query_row(
                "SELECT state FROM exchanges WHERE ulid = ?1",
                [ex.id().to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(state, "committed");
    }
    store.close().unwrap();

    // The same file again (a crash between COMMIT and unlink) is a no-op.
    std::fs::write(
        dir.join("journal").join(format!("{}.json", a.id())),
        serde_json::to_vec(&a).unwrap(),
    )
    .unwrap();
    let store = Store::open(Project::open(&dir).unwrap(), cfg()).unwrap();
    assert_eq!(journal_files(&dir), 0);
    let conn = store.reader().unwrap();
    let (n, refs): (i64, i64) = conn
        .query_row(
            "SELECT (SELECT count(*) FROM exchanges), (SELECT sum(refcount) FROM blobs)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((n, refs), (2, 1));
    store.close().unwrap();
}

#[test]
fn bad_journal_file_is_quarantined() {
    let tmp = tempfile::tempdir().unwrap();
    let (dir, store) = new_store(&tmp, cfg());
    let mut ev = store.subscribe();
    let lost = Ulid::new();
    let name = format!("{lost}.json");
    std::fs::write(dir.join("journal").join(&name), b"not json").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut saw_error = false;
    while Instant::now() < deadline && !saw_error {
        match ev.try_recv() {
            Some(Event::Error { detail, .. }) => saw_error = detail.contains(&name),
            Some(_) => {}
            None => std::thread::sleep(Duration::from_millis(5)),
        }
    }
    assert!(saw_error, "an Error event names the file");
    assert!(dir.join("journal").join("bad").join(&name).is_file());
    assert_eq!(journal_files(&dir), 0);
    // The writer is still alive.
    let ex = exchange(b"after", b"after");
    let id = ex.id();
    store.sink().submit(ex).unwrap();
    assert!(wait_committed(&mut ev, &[id], Duration::from_secs(5)).contains(&id));
    store.close().unwrap();
    // The loss has a gap row whose window starts at the file's ULID.
    let gaps = gap_markers(&dir);
    assert_eq!(gaps.len(), 1, "one gap row for the unreadable file");
    let from = gaps[0].from.expect("a gap window start");
    assert!(from <= lost && lost <= gaps[0].to, "{lost} outside the gap");
}

#[cfg(unix)]
#[test]
fn unreadable_journal_file_is_kept_and_replayed_once_readable() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let (dir, store) = new_store(&tmp, cfg());
    store.close().unwrap();
    let ex = exchange(b"held", b"held");
    let id = ex.id();
    let path = dir.join("journal").join(format!("{id}.json"));
    std::fs::write(&path, serde_json::to_vec(&ex).unwrap()).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::File::open(&path).is_ok() {
        eprintln!("skipped: running as root, file permissions are not enforced");
        return;
    }

    let store = Store::open(Project::open(&dir).unwrap(), cfg()).unwrap();
    let mut ev = store.subscribe();
    assert!(
        path.exists(),
        "a transient read error leaves the file in place"
    );
    assert!(!dir.join("journal/bad").join(format!("{id}.json")).exists());
    assert!(
        store
            .open_report()
            .events
            .iter()
            .any(|e| matches!(e, Event::Error { .. })),
        "the read failure is reported"
    );
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(
        wait_committed(&mut ev, &[id], Duration::from_secs(10)).contains(&id),
        "replayed once readable"
    );
    store.close().unwrap();
    assert!(!path.exists());
    assert!(gap_markers(&dir).is_empty(), "nothing was lost");
}

#[test]
fn journal_cap_drops_with_error_and_gap_marker() {
    let tmp = tempfile::tempdir().unwrap();
    let pause = Arc::new(Mutex::new(()));
    let (dir, store) = new_store(
        &tmp,
        StoreConfig {
            queue_max: 1,
            journal_cap: 1,
            test_pause: Some(Arc::clone(&pause)),
            ..cfg()
        },
    );
    let mut ev = store.subscribe();
    let sink = store.sink();
    let guard = pause.lock().unwrap();
    sink.submit(exchange(b"1", b"1")).unwrap();
    let (two, three) = (exchange(b"2", b"2"), exchange(b"3", b"3"));
    let dropped = [two.id(), three.id()];
    assert_eq!(sink.submit(two), Err(SinkError::Full));
    assert_eq!(sink.submit(three), Err(SinkError::Full));
    let mut errors = 0;
    while let Some(e) = ev.try_recv() {
        match e {
            Event::Error { .. } => errors += 1,
            Event::Received { id, .. } => {
                assert!(!dropped.contains(&id), "no Received for a dropped exchange");
            }
            _ => {}
        }
    }
    assert_eq!(errors, 1, "one Error event per episode");
    assert_eq!(journal_files(&dir), 0);
    drop(guard);
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut marker: Option<Marker> = None;
    while Instant::now() < deadline && marker.is_none() {
        match ev.try_recv() {
            Some(Event::Marker { marker: m, .. }) => marker = Some(m),
            Some(_) => {}
            None => std::thread::sleep(Duration::from_millis(5)),
        }
    }
    let marker = marker.expect("a gap marker event");
    assert_eq!(marker.kind(), crosswire_model::MarkerKind::Gap);
    store.close().unwrap();
    let gaps = gap_markers(&dir);
    assert_eq!(gaps.len(), 1);
    // The window brackets both dropped exchanges.
    let from = gaps[0].from.expect("a drop window has a start");
    for id in dropped {
        assert!(
            from <= id && id <= gaps[0].to,
            "{id} outside the gap window"
        );
    }
}

#[test]
fn closed_sink_refuses() {
    let tmp = tempfile::tempdir().unwrap();
    let (_dir, store) = new_store(&tmp, cfg());
    let sink = store.sink();
    store.close().unwrap();
    assert_eq!(sink.submit(exchange(b"a", b"a")), Err(SinkError::Closed));
}

#[test]
fn http1_heads_fill_search_columns() {
    let tmp = tempfile::tempdir().unwrap();
    let (_dir, store) = new_store(&tmp, cfg());
    let mut ev = store.subscribe();
    let req_head = b"POST /login?x=1 HTTP/1.1\r\nHost: Shop.Example.TEST\r\n\r\n";
    let resp_head = b"HTTP/1.1 302 Found\r\n\r\n";
    let req = Message::new(
        MessageKind::Http1Raw,
        Fidelity::Raw,
        Head::Http1Raw {
            bytes: Bytes::from_static(req_head),
            spans: HeadSpans::new(
                StartLineSpans::Request {
                    method: Span::new(0, 4),
                    target: Span::new(5, 15),
                    version: Span::new(16, 24),
                },
                vec![HeaderSpanRecord::new(
                    Span::new(26, 30),
                    Span::new(32, 49),
                    Span::new(26, 49),
                )],
                FramingRecord::None,
            ),
            parser_version: HTTP1_PARSER_VERSION,
        },
        FramingRecord::None,
        Body::None,
        ConnectionInfo::new("203.0.113.9", 443),
    );
    let resp = Message::new(
        MessageKind::Http1Raw,
        Fidelity::Raw,
        Head::Http1Raw {
            bytes: Bytes::from_static(resp_head),
            spans: HeadSpans::new(
                StartLineSpans::Response {
                    version: Span::new(0, 8),
                    status: Span::new(9, 12),
                    reason: Span::new(13, 18),
                },
                Vec::new(),
                FramingRecord::None,
            ),
            parser_version: HTTP1_PARSER_VERSION,
        },
        FramingRecord::ContentLength { len: 5 },
        Body::Inline {
            bytes: Bytes::from_static(b"hello"),
        },
        ConnectionInfo::new("example.test", 443),
    );
    let mut ex = Exchange::new(req);
    ex.set_response(resp);
    ex.add_tag("login");
    ex.set_notes("note");
    let id = ex.id();
    store.sink().submit(ex).unwrap();
    assert!(wait_committed(&mut ev, &[id], Duration::from_secs(5)).contains(&id));
    let conn = store.reader().unwrap();
    let row: (
        String,
        String,
        String,
        i64,
        i64,
        String,
        String,
        String,
        String,
    ) = conn
        .query_row(
            "SELECT req_method, req_host, req_path, resp_status, resp_len, namespace, \
             scope_state, tags, notes FROM exchanges WHERE ulid = ?1",
            [id.to_string()],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(row.0, "POST");
    assert_eq!(
        row.1, "shop.example.test",
        "from the Host header, not the target"
    );
    assert_eq!(row.2, "/login?x=1");
    assert_eq!((row.3, row.4), (302, 5));
    assert_eq!((row.5.as_str(), row.6.as_str()), ("history", "empty"));
    assert_eq!(row.7, "[\"login\"]");
    assert_eq!(row.8, "note");
    store.close().unwrap();
}

#[test]
fn store_and_stream_are_shareable() {
    fn send_sync<T: Send + Sync>() {}
    fn send<T: Send>() {}
    send_sync::<Store>();
    send::<EventStream>();
}

/// Writes a journal file the way the sink does (complete before visible).
fn plant_journal(dir: &Path, ex: &Exchange) -> u64 {
    let data = serde_json::to_vec(ex).unwrap();
    let tmp = dir.join("journal").join(format!("{}.json.tmp", ex.id()));
    std::fs::write(&tmp, &data).unwrap();
    std::fs::rename(&tmp, dir.join("journal").join(format!("{}.json", ex.id()))).unwrap();
    data.len() as u64
}

#[test]
fn drop_commits_memory_but_leaves_the_journal() {
    let tmp = tempfile::tempdir().unwrap();
    let (dir, store) = new_store(
        &tmp,
        StoreConfig {
            batch_interval: Duration::from_secs(10),
            ..StoreConfig::default()
        },
    );
    let queued = exchange(b"queued", b"q");
    let planted: Vec<Exchange> = (0..3u64)
        .map(|i| exchange(&noise(10, i), &noise(10, i + 9)))
        .collect();
    store.sink().submit(queued.clone()).unwrap();
    for ex in &planted {
        plant_journal(&dir, ex);
    }
    let start = Instant::now();
    drop(store);
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "drop does not wait on the journal"
    );
    assert_eq!(
        journal_files(&dir),
        3,
        "the journal is left for the next open"
    );

    let store = Store::open(Project::open(&dir).unwrap(), cfg()).unwrap();
    assert_eq!(store.open_report().journal_files_drained, 3);
    assert!(store.open_report().unclean_gap.is_some());
    let conn = store.reader().unwrap();
    for ex in planted.iter().chain([&queued]) {
        let n: i64 = conn
            .query_row(
                "SELECT count(*) FROM exchanges WHERE ulid = ?1",
                [ex.id().to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "{} stored", ex.id());
    }
    drop(conn);
    store.close().unwrap();
}

#[test]
fn journal_drain_respects_the_byte_budget() {
    let tmp = tempfile::tempdir().unwrap();
    let pause = Arc::new(Mutex::new(()));
    let (dir, store) = new_store(
        &tmp,
        StoreConfig {
            queue_bytes_max: 100 * 1024,
            test_pause: Some(Arc::clone(&pause)),
            ..cfg()
        },
    );
    let mut ev = store.subscribe();
    // All three files are in place before the writer looks.
    let guard = pause.lock().unwrap();
    let exs: Vec<Exchange> = (0..3u64)
        .map(|i| exchange(&noise(40 * 1024, i), b"r"))
        .collect();
    for ex in &exs {
        // Each file is over half the budget, so each batch holds one.
        assert!(plant_journal(&dir, ex) > 50 * 1024);
    }
    drop(guard);
    let want: Vec<Ulid> = exs.iter().map(Exchange::id).collect();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut got = BTreeSet::new();
    while Instant::now() < deadline && got.len() < want.len() {
        match ev.try_recv() {
            Some(Event::Committed { ids, .. }) => {
                assert_eq!(ids.len(), 1, "one large journal file per batch");
                got.extend(ids);
            }
            Some(_) => {}
            None => std::thread::sleep(Duration::from_millis(2)),
        }
    }
    assert_eq!(got, want.iter().copied().collect());
    store.close().unwrap();
}

#[test]
fn open_report_lists_what_open_did() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("p");
    let p = Project::create(&dir).unwrap();
    let good = exchange(b"good", b"good");
    plant_journal(&dir, &good);
    let bad = format!("{}.json", Ulid::new());
    std::fs::write(dir.join("journal").join(&bad), b"{ broken").unwrap();
    let store = Store::open(p, cfg()).unwrap();
    let report = store.open_report();
    assert_eq!(report.journal_files_drained, 2);
    assert!(
        report.unclean_gap.is_none(),
        "a new project was not unclean"
    );
    assert!(report
        .events
        .iter()
        .any(|e| matches!(e, Event::Error { detail, .. } if detail.contains(&bad))));
    assert!(report
        .events
        .iter()
        .any(|e| matches!(e, Event::Committed { ids, .. } if ids.contains(&good.id()))));
    store.close().unwrap();
}
