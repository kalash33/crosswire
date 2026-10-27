//! Gate §13 #16 — the hostile-project corpus: a malicious `project.db` schema
//! and a malformed `journal/`. A Crosswire project directory may be a file an
//! attacker handed us; opening it must not run foreign code, panic, or let an
//! attacker-controlled length drive an allocation.

use crosswire_model::{
    Body, ConnectionInfo, Exchange, Fidelity, FramingRecord, Head, Message, MessageKind,
};
use crosswire_store::{Project, Store, StoreConfig, StoreError};
use rusqlite::Connection;
use std::time::{Duration, Instant};

/// A raw (non-Crosswire) connection to the project database, as an attacker's
/// own tooling would open it: no defensive mode, no hardening.
fn raw(dir: &std::path::Path) -> Connection {
    Connection::open(dir.join("project.db")).unwrap()
}

fn sentinel_rows(dir: &std::path::Path) -> i64 {
    raw(dir)
        .query_row("SELECT count(*) FROM pwned", [], |r| r.get(0))
        .unwrap_or(-1)
}

/// A minimal, serializable exchange (as a journal spill holds).
fn exchange() -> Exchange {
    let msg = Message::new(
        MessageKind::Normalized,
        Fidelity::Normalized,
        Head::Normalized { map: Vec::new() },
        FramingRecord::None,
        Body::inline(bytes::Bytes::from_static(b"hello")),
        ConnectionInfo::new("example.test", 443),
    );
    Exchange::new(msg)
}

/// A hostile `project.db` carries valid Crosswire headers but plants a trigger
/// (and a view) on our own tables. Opening the project must refuse it with a
/// typed error and run none of the attacker's SQL — no sentinel row, no panic.
#[test]
fn hostile_sqlite_schema_does_not_execute_on_open() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("p");
    // A genuine Crosswire project: correct application_id and user_version, so
    // the open gets past the header checks and reaches the schema.
    drop(Project::create(&dir).unwrap());
    // The attacker injects schema objects Crosswire never creates: a sentinel
    // table, a trigger that runs on every write to `exchanges` (a journal
    // replay would write there), and a view whose body calls a dangerous
    // function.
    {
        let c = raw(&dir);
        c.execute_batch(
            "CREATE TABLE pwned(hit TEXT);
             CREATE TRIGGER evil_exchanges AFTER INSERT ON exchanges \
                 BEGIN INSERT INTO pwned VALUES ('exchanges'); END;
             CREATE TRIGGER evil_markers AFTER INSERT ON markers \
                 BEGIN INSERT INTO pwned VALUES ('markers'); END;
             CREATE VIEW evil_view AS SELECT load_extension('/tmp/evil') AS p;",
        )
        .unwrap();
    }

    // The project-open path refuses the foreign schema with a typed error,
    // never a panic. `Store::open` consumes a `Project`, so refusing here
    // closes the store path too.
    let err = Project::open(&dir).unwrap_err();
    assert!(
        matches!(&err, StoreError::Corrupt(m) if m.contains("trigger or view")),
        "hostile schema must be a typed refusal, got {err:?}"
    );
    // Opening ran none of the attacker's SQL: the sentinel is still empty.
    assert_eq!(
        sentinel_rows(&dir),
        0,
        "opening the project fired an attacker trigger"
    );

    // Non-vacuity: the trigger is live. A write to `exchanges` — exactly what a
    // journal replay or gap marker does — fires it, so the refusal above is
    // what kept it from running, not an inert schema.
    {
        let c = raw(&dir);
        c.execute(
            "INSERT INTO exchanges (ulid, namespace, state, scope_state, created_at, json) \
             VALUES ('01ARZ3NDEKTSV4RRFFQ69G5FAV', 'default', 'complete', 'in', 0, '{}')",
            [],
        )
        .unwrap();
    }
    assert_eq!(
        sentinel_rows(&dir),
        1,
        "the planted trigger does fire on a write to exchanges"
    );
}

/// Writes `bytes` to `journal/<name>` under the project directory.
fn plant_journal(dir: &std::path::Path, name: &str, bytes: &[u8]) {
    std::fs::write(dir.join("journal").join(name), bytes).unwrap();
}

/// A malformed `journal/` must not crash the startup drain or let a huge file
/// size drive an allocation: good records still replay, every bad record is
/// quarantined, and the whole open stays prompt and bounded.
#[test]
fn malformed_journal_is_skipped_without_panic_or_unbounded_alloc() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("p");
    drop(Project::create(&dir).unwrap());

    // (good) one valid exchange, as a real spill holds.
    let good = exchange();
    plant_journal(
        &dir,
        &format!("{}.json", good.id()),
        &serde_json::to_vec(&good).unwrap(),
    );
    // (a) truncated / garbage bytes.
    plant_journal(&dir, "01BRZ3NDEKTSV4RRFFQ69G5FA1.json", b"{ not valid json");
    // (b) an attacker-chosen huge size. The journal reader bounds a file by its
    //     size (a JSON spill has no length header of its own), so a sparse file
    //     that *claims* to be larger than the 256 MiB cap is the hostile length
    //     field. It must be rejected by the size check, never read into a
    //     size-of-file buffer. A sparse `set_len` costs no real bytes.
    const OVER_CAP: u64 = 256 * 1024 * 1024 + 1;
    {
        let f = std::fs::File::create(dir.join("journal").join("01CRZ3NDEKTSV4RRFFQ69G5FA2.json"))
            .unwrap();
        f.set_len(OVER_CAP).unwrap();
    }
    // (c) well-formed JSON that is not an Exchange.
    plant_journal(
        &dir,
        "01DRZ3NDEKTSV4RRFFQ69G5FA3.json",
        br#"{"id":"not-a-ulid","nonsense":true}"#,
    );

    let started = Instant::now();
    let store = Store::open(Project::open(&dir).unwrap(), StoreConfig::default()).unwrap();
    let elapsed = started.elapsed();
    // Bounded: had the reader allocated the claimed file size (or read the
    // whole sparse file), the open would have stalled or run out of memory.
    assert!(
        elapsed < Duration::from_secs(30),
        "drain was not prompt ({elapsed:?}); a huge file may have driven an allocation"
    );
    // Every one of the four files left `journal/` (one committed, three
    // quarantined) and none crashed the drain.
    assert_eq!(store.open_report().journal_files_drained, 4);

    let dir = store.dir().to_path_buf();
    store.close().unwrap();

    // The good record replayed into the database.
    let conn = Connection::open(dir.join("project.db")).unwrap();
    let committed: i64 = conn
        .query_row(
            "SELECT count(*) FROM exchanges WHERE ulid = ?1",
            [good.id().to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(committed, 1, "the good journal record must replay");

    // The three bad records were quarantined, and none linger in `journal/`.
    let bad = std::fs::read_dir(dir.join("journal").join("bad"))
        .unwrap()
        .count();
    assert_eq!(bad, 3, "every malformed record must be quarantined");
    let remaining = std::fs::read_dir(dir.join("journal"))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("json"))
        })
        .count();
    assert_eq!(remaining, 0, "no journal record left behind");
}
