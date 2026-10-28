//! §13 gate #18 flat-RSS for the store's memory-relevant Limits rows: the
//! write queue and pack-body decompression.
//!
//! Each test drives hostile input far past the cap and asserts the observable
//! refusal AND that anonymous RSS stays flat. The RSS assertion runs where the
//! probe exists (Linux `RssAnon`, macOS `phys_footprint`); elsewhere it SKIPs.
//! The attacker volume is never held by the test: the queue flood clones one
//! shared body per exchange, and the pack bomb is a few dozen bytes on disk.

#[path = "../src/rss.rs"]
mod rss;

use bytes::Bytes;
use crosswire_model::{
    Body, ConnectionInfo, Exchange, Fidelity, FramingRecord, Head, Message, MessageKind, SinkError,
};
use crosswire_store::{BodySide, BodyStore, PackWriter, Project, Store, StoreConfig, StoreError};
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use ulid::Ulid;

/// RSS is process-wide: only one measured section at a time.
static GUARD: Mutex<()> = Mutex::new(());

/// Runs `drive`, sampling anonymous RSS on a background thread, and asserts the
/// peak grew by less than `cap_mib` MiB over the baseline.
fn with_flat_rss<R>(cap_mib: u64, drive: impl FnOnce() -> R) -> R {
    let _g = GUARD.lock().unwrap_or_else(|e| e.into_inner());
    let Some(baseline) = rss::anon_rss_bytes() else {
        eprintln!("flat-RSS: probe unavailable on this host; SKIP");
        return drive();
    };
    let peak = Arc::new(AtomicU64::new(baseline));
    let stop = Arc::new(AtomicBool::new(false));
    let sampler = {
        let (peak, stop) = (Arc::clone(&peak), Arc::clone(&stop));
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                if let Some(b) = rss::anon_rss_bytes() {
                    peak.fetch_max(b, Ordering::Relaxed);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        })
    };
    let out = drive();
    if let Some(b) = rss::anon_rss_bytes() {
        peak.fetch_max(b, Ordering::Relaxed);
    }
    stop.store(true, Ordering::Relaxed);
    sampler.join().unwrap();
    let growth = peak.load(Ordering::Relaxed).saturating_sub(baseline);
    let cap = cap_mib * 1024 * 1024;
    assert!(
        growth < cap,
        "flat-RSS exceeded: baseline {baseline} B, growth {growth} B >= cap {cap} B"
    );
    out
}

fn msg(body: Bytes) -> Message {
    Message::new(
        MessageKind::Normalized,
        Fidelity::Normalized,
        Head::Normalized { map: Vec::new() },
        FramingRecord::ContentLength {
            len: body.len() as u64,
        },
        Body::inline(body),
        ConnectionInfo::new("example.test", 443),
    )
}

/// 128 MiB of exchanges against a 4-exchange queue and a 16 MiB journal cap,
/// with the writer stalled: the queue spills to the journal, then the journal
/// cap drops with `Full` and a gap marker; RSS tracks the queue, not the flood.
#[test]
fn store_queue_flat_rss() {
    const FLOOD: usize = 128;
    const BODY: usize = 1024 * 1024;
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("p");
    let pause = Arc::new(Mutex::new(()));
    let store = Store::open(
        Project::create(&dir).unwrap(),
        StoreConfig {
            batch_interval: Duration::from_millis(20),
            queue_max: 4,
            journal_cap: 16 * 1024 * 1024,
            test_pause: Some(Arc::clone(&pause)),
            ..StoreConfig::default()
        },
    )
    .unwrap();
    let sink = store.sink();
    // One shared allocation: each exchange clones the handle, not the bytes.
    let shared = Bytes::from(vec![0xA5u8; BODY]);
    let stall = pause.lock().unwrap();
    let (accepted, dropped) = with_flat_rss(64, || {
        let (mut ok, mut full) = (0usize, 0usize);
        for _ in 0..FLOOD {
            match sink.submit(Exchange::new(msg(shared.clone()))) {
                Ok(_) => ok += 1,
                Err(SinkError::Full) => full += 1,
                Err(e) => panic!("unexpected {e:?}"),
            }
        }
        (ok, full)
    });
    assert!(dropped > 0, "the journal cap must refuse part of the flood");
    assert!(
        accepted < FLOOD / 2,
        "accepted {accepted} of {FLOOD}: far fewer than the flood fit the caps"
    );
    let journal_bytes: u64 = std::fs::read_dir(dir.join("journal"))
        .unwrap()
        .map(|e| e.unwrap().metadata().unwrap().len())
        .sum();
    assert!(
        journal_bytes < 64 * 1024 * 1024,
        "journal {journal_bytes} B grew past its cap (+ one exchange of slack)"
    );
    drop(stall);
    store.close().unwrap();
    let conn = rusqlite::Connection::open_with_flags(
        dir.join("project.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let gaps: i64 = conn
        .query_row("SELECT count(*) FROM markers WHERE kind = 'gap'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(gaps >= 1, "dropped exchanges must leave a gap marker");
}

/// Hand-built zstd frame: single-segment header declaring `declared` bytes of
/// content, then one empty raw last block. Under 20 bytes on disk.
fn bomb_frame(declared: u64) -> Vec<u8> {
    let mut f = vec![0x28, 0xB5, 0x2F, 0xFD, 0xE0];
    f.extend(declared.to_le_bytes());
    f.extend([0x01, 0x00, 0x00]);
    f
}

fn plant_bomb(p: &mut Project, declared: u64, ulid: Ulid) {
    let frame = bomb_frame(declared);
    let hash = [7u8; 32];
    let path = p.pack_path(1);
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    let offset = f.metadata().unwrap().len();
    let mut rec = b"CWPK".to_vec();
    rec.extend((frame.len() as u32).to_le_bytes());
    rec.extend(hash);
    rec.extend(&frame);
    f.write_all(&rec).unwrap();
    let tx = p.writer_mut().transaction().unwrap();
    tx.execute(
        "INSERT INTO exchanges (ulid, namespace, state, scope_state, created_at, json) \
         VALUES (?1, 'default', 'complete', 'in', 0, '{}')",
        [ulid.to_string()],
    )
    .unwrap();
    tx.execute(
        "INSERT INTO blobs (hash, pack_id, offset, len) VALUES (?1, 1, ?2, ?3)",
        rusqlite::params![hash.as_slice(), offset as i64, rec.len() as i64],
    )
    .unwrap();
    tx.execute(
        "INSERT INTO bodies (ulid, side, blob_hash, len) VALUES (?1, 'resp', ?2, 0)",
        rusqlite::params![ulid.to_string(), hash.as_slice()],
    )
    .unwrap();
    tx.commit().unwrap();
}

/// A pack body declaring 16 GiB (256x the 64 MiB bound) is refused as Corrupt
/// before any decompression buffer is allocated; repeated reads keep RSS flat.
#[test]
fn pack_decompression_flat_rss() {
    let tmp = tempfile::tempdir().unwrap();
    let mut p = Project::create(&tmp.path().join("p")).unwrap();
    let _bodies = BodyStore::new(PackWriter::open(&mut p).unwrap());
    let id = Ulid::new();
    plant_bomb(&mut p, 16 * 1024 * 1024 * 1024, id);
    let conn = p.reader().unwrap();
    with_flat_rss(32, || {
        for _ in 0..200 {
            let err = BodyStore::get(&conn, p.dir(), id, BodySide::Resp)
                .expect_err("a decompression bomb must be refused");
            let StoreError::Corrupt(m) = err else {
                panic!("expected Corrupt, got {err:?}")
            };
            assert!(m.contains("over limit"), "{m}");
        }
    });
}
