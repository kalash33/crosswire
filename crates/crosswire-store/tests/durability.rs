//! The durability promise (spec section 9.3) under SIGKILL.
#![cfg(unix)]

use crosswire_store::{sweep, BodySide, BodyStore, Project, Store, StoreConfig};
use std::collections::BTreeSet;
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
    let lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let mut committed: Vec<Ulid> = Vec::new();
    let mut accepted: Vec<Ulid> = Vec::new();
    // Accepted ids printed after the last `committed` line read.
    let mut after_last_commit: BTreeSet<Ulid> = BTreeSet::new();
    let mut killed = false;
    // Read to EOF: lines printed before the kill are still in the pipe.
    for line in lines {
        let line = line.unwrap();
        if let Some(rest) = line.strip_prefix("committed") {
            for tok in rest.split_whitespace() {
                committed.push(Ulid::from_string(tok).unwrap());
            }
            after_last_commit.clear();
        } else if let Some(rest) = line.strip_prefix("accepted ") {
            let id = Ulid::from_string(rest.trim()).unwrap();
            accepted.push(id);
            after_last_commit.insert(id);
        }
        if !killed && committed.len() >= KILL_AFTER {
            child.kill().unwrap(); // SIGKILL
            killed = true;
        }
    }
    let status = child.wait().unwrap();
    assert!(
        killed,
        "child stopped early ({status}) after {} committed ids",
        committed.len()
    );
    let spilled = std::fs::read_dir(dir.join("journal"))
        .unwrap()
        .filter(|e| {
            let e = e.as_ref().unwrap();
            e.file_name().to_string_lossy().ends_with(".json")
        })
        .count();

    // Reopen: the sweep and an integrity check must pass.
    let mut p = Project::open(&dir).unwrap();
    sweep(&mut p).unwrap();
    let check: String = p
        .writer_mut()
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(check, "ok");

    // Opening the store records the gap, then replays the journal.
    let store = Store::open(p, StoreConfig::default()).unwrap();
    let gap = store
        .open_report()
        .unclean_gap
        .clone()
        .expect("an unclean-restart gap");
    let conn = store.reader().unwrap();
    let stored = |id: &Ulid| -> Option<(String, String)> {
        conn.query_row(
            "SELECT state, notes FROM exchanges WHERE ulid = ?1",
            [id.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok()
    };
    let check_bodies = |id: &Ulid, notes: &str| {
        let index: u64 = notes.parse().unwrap();
        for (side, n) in [(BodySide::Req, 0), (BodySide::Resp, 1)] {
            let body = BodyStore::get(&conn, &dir, *id, side).unwrap().unwrap();
            assert!(
                body == pattern(index, n, BODY_LEN),
                "{id} {side:?} body differs"
            );
        }
    };
    // Every id seen committed is there, byte-exact.
    for id in &committed {
        let (state, notes) = stored(id).unwrap_or_else(|| panic!("committed {id} lost"));
        assert_eq!(state, "committed", "{id}");
        check_bodies(id, &notes);
    }
    // Every accepted id is either there and byte-exact, or inside the gap window.
    let from = gap.from_ulid().expect("the previous run committed");
    let to = gap.to_ulid().expect("a restart gap ends at the restart");
    let mut missing = Vec::new();
    for id in &accepted {
        match stored(id) {
            Some((state, notes)) => {
                assert_eq!(state, "committed", "{id}");
                check_bodies(id, &notes);
            }
            None => missing.push(*id),
        }
    }
    for id in &missing {
        assert!(
            *id >= from,
            "missing {id} is before the gap window starting {from}"
        );
        assert!(*id < to, "missing {id} is after the gap window ending {to}");
    }
    let missing_before_last_commit = missing
        .iter()
        .filter(|id| !after_last_commit.contains(id))
        .count();
    eprintln!(
        "killed after {} committed ids seen; {} accepted; {spilled} in journal/; \
         {} missing ({missing_before_last_commit} accepted before the last committed line)",
        committed.len(),
        accepted.len(),
        missing.len()
    );
    let gaps: i64 = conn
        .query_row("SELECT count(*) FROM markers WHERE kind = 'gap'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(gaps, 1, "exactly one gap marker");
    drop(conn);
    store.close().unwrap();

    // The packs validate end to end after recovery.
    let mut p = Project::open(&dir).unwrap();
    let report = sweep(&mut p).unwrap();
    assert_eq!(report.truncated_bytes, 0);
    assert_eq!(report.exchanges_marked, 0);
}
