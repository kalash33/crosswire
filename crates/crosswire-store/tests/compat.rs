//! On-disk compatibility: a project written by 0.1.0 opens and reads back
//! unchanged under the SQLite this build bundles.
//!
//! `tests/fixtures/project-0.1.0/` is a small project written once by
//! [`write_0_1_0_fixture`] (an `#[ignore]`d generator) on the 0.1.0
//! dependencies: rusqlite 0.32 with bundled SQLite 3.46.0. It was closed
//! cleanly, so the WAL is checkpointed and truncated; the lock file, `-wal`,
//! `-shm` and `audit.jsonl` are not committed. `expected.json` beside it
//! records what 0.1.0 read back: every exchange's ULID and the SHA-256 of its
//! JSON (as stored, and as [`Store::get`] returns it), every body's SHA-256,
//! every blob hash, the markers, the result of each structured filter, the
//! token hash, and the header values.
//!
//! [`opens_0_1_0_project`] copies the fixture to a temporary directory (the
//! committed files are never opened in place) and checks that this build
//! opens it without a migration or `.bak` snapshot, that SQLite finds it
//! intact, and that every recorded value reads back the same. It then appends
//! an exchange and closes cleanly. A bundled-SQLite upgrade must keep this
//! test passing unchanged.

use bytes::Bytes;
use crosswire_h1::{Parser, ParserConfig, Progress, ResponseContext};
use crosswire_model::{
    Body, Capture, ConnectionInfo, Event, Exchange, Message, NamespaceKind, ScopeState,
};
use crosswire_store::{
    BodySide, EventStream, ExchangeFilter, Project, Store, StoreConfig, TokenScope,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use ulid::Ulid;

/// The SQLite that wrote the fixture. The generator refuses to run on any
/// other, so the fixture always stands for what 0.1.0 wrote.
const WRITER_SQLITE: &str = "3.46.0";

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/project-0.1.0")
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// A fixed id: the same fixture bytes on every run of the generator.
fn id(n: u64) -> Ulid {
    Ulid::from_parts(1_762_000_000_000 + n * 1000, u128::from(n) * 0x1_0000_0001)
}

fn req(method: &str, host: &str, path: &str, body: Body) -> Message {
    let head = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\n\r\n");
    let mut p = Parser::request(&ParserConfig::default());
    let Progress::Head(raw) = p.feed(head.as_bytes()).unwrap().event else {
        panic!("request head");
    };
    Message::http1_raw(&raw, body, ConnectionInfo::new(host, 443))
}

fn resp(method: &str, status: u16, body: Body) -> Message {
    let head = format!("HTTP/1.1 {status} X\r\n\r\n");
    let mut p = Parser::response(
        &ParserConfig::default(),
        ResponseContext::for_method(method.as_bytes()),
    );
    let Progress::Head(raw) = p.feed(head.as_bytes()).unwrap().event else {
        panic!("response head");
    };
    Message::http1_raw(&raw, body, ConnectionInfo::new("resp.test", 443))
}

fn exchange(
    n: u64,
    method: &str,
    host: &str,
    status: u16,
    req_body: Body,
    resp_body: Body,
) -> Exchange {
    let mut ex = Exchange::with_id(id(n), req(method, host, "/", req_body));
    ex.set_response(resp(method, status, resp_body));
    ex
}

/// A body above the 16 KiB inline limit, so it is stored in a pack; text, so
/// the committed pack stays small.
fn pack_body() -> Bytes {
    let text: String = (0..800)
        .map(|i| format!("crosswire fixture {i:04}\n"))
        .collect();
    assert!(text.len() > 16 * 1024);
    Bytes::from(text)
}

fn wait_committed(ev: &mut EventStream, want: &[Ulid]) {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut got = BTreeSet::new();
    while Instant::now() < deadline && !want.iter().all(|w| got.contains(w)) {
        match ev.try_recv() {
            Some(Event::Committed { ids, .. }) => got.extend(ids),
            Some(_) => {}
            None => std::thread::sleep(Duration::from_millis(2)),
        }
    }
    assert!(want.iter().all(|w| got.contains(w)), "not all committed");
}

fn submit(store: &Store, exs: Vec<Exchange>) -> Vec<Ulid> {
    let mut ev = store.subscribe();
    let ids: Vec<Ulid> = exs.iter().map(Exchange::id).collect();
    let sink = store.sink();
    for ex in exs {
        sink.submit(ex).unwrap();
    }
    wait_committed(&mut ev, &ids);
    ids
}

fn config() -> StoreConfig {
    StoreConfig {
        batch_interval: Duration::from_millis(10),
        ..StoreConfig::default()
    }
}

/// The structured filters the fixture pins, by name.
fn filters() -> Vec<(&'static str, ExchangeFilter)> {
    vec![
        (
            "host",
            ExchangeFilter {
                host: Some("b.test".into()),
                ..ExchangeFilter::default()
            },
        ),
        (
            "method_status",
            ExchangeFilter {
                method: Some("POST".into()),
                status: Some(500..=599),
                ..ExchangeFilter::default()
            },
        ),
        (
            "resp_len",
            ExchangeFilter {
                min_resp_len: Some(1000),
                ..ExchangeFilter::default()
            },
        ),
        (
            "scope",
            ExchangeFilter {
                scope_state: Some(ScopeState::In),
                ..ExchangeFilter::default()
            },
        ),
        (
            "tag",
            ExchangeFilter {
                tag: Some("login".into()),
                ..ExchangeFilter::default()
            },
        ),
        (
            "namespace",
            ExchangeFilter {
                namespace: Some(NamespaceKind::History),
                ..ExchangeFilter::default()
            },
        ),
    ]
}

fn list_ids(store: &Store, filter: &ExchangeFilter) -> Vec<String> {
    let page = store.list(filter, None, 100).unwrap();
    assert!(page.next_cursor.is_none());
    page.items.iter().map(|r| r.id.to_string()).collect()
}

fn pragma_i64(conn: &rusqlite::Connection, name: &str) -> i64 {
    conn.query_row(&format!("PRAGMA {name}"), [], |r| r.get(0))
        .unwrap()
}

/// What a store reads back from the project, in the shape of `expected.json`
/// (without the provenance fields).
fn snapshot(store: &Store) -> Value {
    let conn = store.reader().unwrap();
    let mut stmt = conn
        .prepare("SELECT ulid, json FROM exchanges ORDER BY ulid")
        .unwrap();
    let rows: Vec<(String, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let mut exchanges = Vec::new();
    for (ulid, stored) in rows {
        let ex_id = Ulid::from_string(&ulid).unwrap();
        let got = store.get(ex_id).unwrap().unwrap();
        let mut entry = json!({
            "ulid": ulid,
            "stored_json_sha256": sha256_hex(stored.as_bytes()),
            "get_json_sha256": sha256_hex(&serde_json::to_vec(&got).unwrap()),
        });
        for (side, key) in [
            (BodySide::Req, "req_body_sha256"),
            (BodySide::Resp, "resp_body_sha256"),
        ] {
            if let Some(bytes) = store.get_body(ex_id, side).unwrap() {
                entry[key] = json!(sha256_hex(&bytes));
            }
        }
        exchanges.push(entry);
    }
    let blobs: Vec<String> = conn
        .prepare("SELECT hash FROM blobs ORDER BY hash")
        .unwrap()
        .query_map([], |r| r.get::<_, Vec<u8>>(0))
        .unwrap()
        .map(|h| hex(&h.unwrap()))
        .collect();
    let markers: Vec<Value> = store
        .markers(None, 100)
        .unwrap()
        .items
        .iter()
        .map(|m| json!({ "ulid": m.id().to_string(), "kind": format!("{:?}", m.kind()) }))
        .collect();
    let mut by_filter = serde_json::Map::new();
    for (name, f) in filters() {
        by_filter.insert(name.into(), json!(list_ids(store, &f)));
    }
    let token_hashes: Vec<String> = conn
        .prepare("SELECT hash FROM tokens ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get::<_, Vec<u8>>(0))
        .unwrap()
        .map(|h| hex(&h.unwrap()))
        .collect();
    json!({
        "application_id": pragma_i64(&conn, "application_id"),
        "user_version": pragma_i64(&conn, "user_version"),
        "exchanges": exchanges,
        "blobs": blobs,
        "markers": markers,
        "filters": by_filter,
        "token_hashes": token_hashes,
    })
}

/// Writes `tests/fixtures/project-0.1.0/`. Run once, on the 0.1.0
/// dependencies only:
/// `cargo test -p crosswire-store --test compat -- --ignored write_0_1_0_fixture`.
#[test]
#[ignore = "writes the committed 0.1.0 fixture; run by hand on the 0.1.0 dependencies only"]
fn write_0_1_0_fixture() {
    assert_eq!(
        rusqlite::version(),
        WRITER_SQLITE,
        "the fixture stands for what 0.1.0 wrote; regenerate it only on 0.1.0's SQLite"
    );
    let dir = fixture_dir();
    if dir.exists() {
        fs::remove_dir_all(&dir).unwrap();
    }
    let big = pack_body();

    // First run: inline, pack, deduplicated and truncated bodies, then a
    // crash (drop without close) so the next open writes a gap marker.
    let store = Store::open(Project::create(&dir).unwrap(), config()).unwrap();
    let mut login = exchange(
        1,
        "GET",
        "a.test",
        200,
        Body::None,
        Body::inline(Bytes::from_static(b"hello")),
    );
    login.add_tag("login");
    login.set_scope_state(ScopeState::In);
    let truncated = Body::Inline {
        bytes: Bytes::from_static(b"partial"),
        truncated_at: Some(7),
        capture_dropped: false,
        relay: Some(Capture {
            len: 100_000,
            sha256: Sha256::digest(b"the whole relayed body").into(),
        }),
    };
    submit(
        &store,
        vec![
            login,
            exchange(
                2,
                "POST",
                "b.test",
                500,
                Body::inline(Bytes::from_static(b"a=1")),
                Body::inline(big.clone()),
            ),
            exchange(3, "GET", "b.test", 200, Body::None, Body::inline(big)),
            exchange(4, "GET", "c.test", 200, Body::None, truncated),
        ],
    );
    drop(store);

    // Second run: gap marker on open, one more exchange, the control tables,
    // and a clean close (checkpoint, WAL truncated, `clean=1`).
    let store = Store::open(Project::open(&dir).unwrap(), config()).unwrap();
    assert!(store.open_report().unclean_gap.is_some());
    let mut put = exchange(
        5,
        "PUT",
        "a.test",
        204,
        Body::inline(Bytes::from_static(b"{\"k\":\"v\"}")),
        Body::None,
    );
    put.set_scope_state(ScopeState::In);
    submit(&store, vec![put]);
    let control = store.control().unwrap();
    assert!(control.exchange_set_tags(id(5), &["admin".into()]).unwrap());
    assert!(control
        .exchange_set_notes(id(2), "server error on login")
        .unwrap());
    control
        .token_create("fixture", &[TokenScope::Read, TokenScope::Send])
        .unwrap();
    control
        .draft_put(id(100), r#"{"raw":"GET / HTTP/1.1"}"#)
        .unwrap();
    control.setting_put("fixture.setting", "on").unwrap();
    drop(control);
    let mut expected = snapshot(&store);
    store.close().unwrap();

    // Only the database, the pack and the empty journal are committed.
    fs::remove_file(dir.join("project.lock")).unwrap();
    fs::remove_file(dir.join("audit.jsonl")).unwrap();
    for ext in ["-wal", "-shm"] {
        let p = dir.join(format!("project.db{ext}"));
        if let Ok(meta) = fs::metadata(&p) {
            assert!(ext == "-shm" || meta.len() == 0, "WAL not truncated");
            fs::remove_file(p).unwrap();
        }
    }
    fs::write(dir.join("journal/.gitkeep"), "").unwrap();
    expected["written_by"] = json!({
        "crosswire": "0.1.0",
        "rusqlite": "0.32.1",
        "sqlite": rusqlite::version(),
    });
    fs::write(
        dir.join("expected.json"),
        serde_json::to_string_pretty(&expected).unwrap() + "\n",
    )
    .unwrap();
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for e in fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dest = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &dest);
        } else {
            fs::copy(e.path(), &dest).unwrap();
        }
    }
}

#[test]
fn opens_0_1_0_project() {
    let fixture = fixture_dir();
    let mut expected: Value =
        serde_json::from_slice(&fs::read(fixture.join("expected.json")).unwrap()).unwrap();
    expected.as_object_mut().unwrap().remove("written_by");
    assert!(
        !fixture.join("project.lock").exists() && !fixture.join("project.db-wal").exists(),
        "the committed fixture must be a cleanly closed project"
    );
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("p");
    copy_dir(&fixture, &dir);
    let before: BTreeSet<_> = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();

    // 1. Opens as a clean format-6 project: no migration, no `.bak`, no gap.
    let store = Store::open(Project::open(&dir).unwrap(), config()).unwrap();
    assert!(store.open_report().unclean_gap.is_none());
    assert_eq!(store.open_report().journal_files_drained, 0);
    let created: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .filter(|n| !before.contains(n))
        .collect();
    assert!(
        created
            .iter()
            .all(|n| !n.to_string_lossy().contains(".bak")),
        "open must not snapshot or migrate: {created:?}"
    );

    // 2. SQLite finds the file intact.
    let conn = store.reader().unwrap();
    let check: String = conn
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(check, "ok");
    assert_eq!(pragma_i64(&conn, "user_version"), 6);
    assert_eq!(pragma_i64(&conn, "application_id"), 0x4357_5052);
    drop(conn);

    // 3-6. Headers, exchanges (stored and as read), bodies, blobs (pack reads
    // verify each record's SHA-256), markers and every structured filter.
    let got = snapshot(&store);
    for key in expected.as_object().unwrap().keys() {
        assert_eq!(
            got[key], expected[key],
            "{key} differs from what 0.1.0 read"
        );
    }
    assert_eq!(got, expected);
    let blobs = expected["blobs"].as_array().unwrap();
    assert!(!blobs.is_empty(), "the fixture holds a pack body");
    for e in expected["exchanges"].as_array().unwrap() {
        let ex_id = Ulid::from_string(e["ulid"].as_str().unwrap()).unwrap();
        if let Some(body) = store.get_body(ex_id, BodySide::Resp).unwrap() {
            if body.len() > 16 * 1024 {
                assert!(blobs.contains(&json!(sha256_hex(&body))));
            }
        }
    }

    // 7. The token is found by its stored hash.
    let control = store.control().unwrap();
    for h in expected["token_hashes"].as_array().unwrap() {
        let h = h.as_str().unwrap();
        let mut hash = [0u8; 32];
        for (i, b) in hash.iter_mut().enumerate() {
            *b = u8::from_str_radix(&h[i * 2..i * 2 + 2], 16).unwrap();
        }
        let rec = control.token_lookup(&hash).unwrap().expect("token found");
        assert_eq!(rec.name, "fixture");
    }
    assert!(control.draft_get(id(100)).unwrap().is_some());
    assert_eq!(
        control.setting_get("fixture.setting").unwrap().as_deref(),
        Some("on")
    );
    drop(control);

    // 8. A new exchange appends, and the project closes and reopens cleanly.
    let new = submit(
        &store,
        vec![exchange(
            6,
            "GET",
            "d.test",
            200,
            Body::None,
            Body::inline(pack_body()),
        )],
    );
    assert!(store.get(new[0]).unwrap().is_some());
    store.close().unwrap();
    assert!(
        fs::metadata(dir.join("project.db-wal")).map_or(0, |m| m.len()) == 0,
        "clean close truncates the WAL"
    );
    let store = Store::open(Project::open(&dir).unwrap(), config()).unwrap();
    assert!(store.open_report().unclean_gap.is_none());
    let conn = store.reader().unwrap();
    let check: String = conn
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(check, "ok");
    let n: i64 = conn
        .query_row("SELECT count(*) FROM exchanges", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        usize::try_from(n).unwrap(),
        expected["exchanges"].as_array().unwrap().len() + 1
    );
    drop(conn);
    store.close().unwrap();
}
