//! The durability promise (spec section 9.3) under SIGKILL.
#![cfg(unix)]

use crosswire_store::{sweep, BodySide, BodyStore, Project, Store, StoreConfig};
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use ulid::Ulid;

/// The body the child writes for exchange `index` on `side` (0 request, 1 response).
/// Must match `src/bin/killtest-child.rs`.
fn pattern(index: u64, side: u64, len: usize) -> Vec<u8> {
    let mut x = (index * 2 + side + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

const BODY_LEN: usize = 20 * 1024;
const TOTAL: u32 = 5000;
const KILL_AFTER: usize = 1000;

#[test]
fn kill_minus_nine() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("p");
    let mut child = Command::new(env!("CARGO_BIN_EXE_killtest-child"))
        .arg(&dir)
        .arg(TOTAL.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    // Keep the pipe open until the kill, so the child never sees EPIPE first.
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let mut seen: Vec<Ulid> = Vec::new();
    for line in lines.by_ref() {
        let line = line.unwrap();
        if let Some(rest) = line.strip_prefix("committed") {
            for tok in rest.split_whitespace() {
                seen.push(Ulid::from_string(tok).unwrap());
            }
        }
        if seen.len() >= KILL_AFTER {
            break;
        }
    }
    child.kill().unwrap(); // SIGKILL
    let status = child.wait().unwrap();
    drop(lines);
    assert!(
        seen.len() >= KILL_AFTER,
        "child stopped early ({status}) after {} committed ids",
        seen.len()
    );
    let last_seen = *seen.iter().max().unwrap();
    let spilled = std::fs::read_dir(dir.join("journal"))
        .unwrap()
        .filter(|e| {
            let e = e.as_ref().unwrap();
            e.file_name().to_string_lossy().ends_with(".json")
        })
        .count();
    eprintln!(
        "killed after {} committed ids seen; {spilled} exchanges in journal/",
        seen.len()
    );

    // Reopen: the sweep and an integrity check must pass.
    let mut p = Project::open(&dir).unwrap();
    sweep(&mut p).unwrap();
    let check: String = p
        .writer_mut()
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(check, "ok");

    // Opening the store replays the journal and records the gap.
    let store = Store::open(p, StoreConfig::default()).unwrap();
    let conn = store.reader().unwrap();
    for id in &seen {
        let (state, notes): (String, String) = conn
            .query_row(
                "SELECT state, notes FROM exchanges WHERE ulid = ?1",
                [id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap_or_else(|e| panic!("committed {id} lost: {e}"));
        assert_eq!(state, "committed", "{id}");
        let index: u64 = notes.parse().unwrap();
        for (side, n) in [(BodySide::Req, 0), (BodySide::Resp, 1)] {
            let body = BodyStore::get(&conn, &dir, *id, side).unwrap().unwrap();
            assert!(
                body == pattern(index, n, BODY_LEN),
                "{id} {side:?} body differs"
            );
        }
    }
    let gaps: Vec<Option<String>> = {
        let mut st = conn
            .prepare("SELECT from_ulid FROM markers WHERE kind = 'gap'")
            .unwrap();
        let rows = st.query_map([], |r| r.get(0)).unwrap();
        rows.map(Result::unwrap).collect()
    };
    assert_eq!(gaps.len(), 1, "exactly one gap marker: {gaps:?}");
    let from = Ulid::from_string(gaps[0].as_deref().expect("gap has a from_ulid")).unwrap();
    assert!(
        from >= last_seen,
        "gap starts at or after the last committed id seen"
    );
    let from_is_committed: bool = conn
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM exchanges WHERE ulid = ?1 AND state = 'committed')",
            [from.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert!(from_is_committed);
    drop(conn);
    store.close().unwrap();

    // The packs validate end to end after recovery.
    let mut p = Project::open(&dir).unwrap();
    let report = sweep(&mut p).unwrap();
    assert_eq!(report.truncated_bytes, 0);
    assert_eq!(report.exchanges_marked, 0);
}
