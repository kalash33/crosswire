//! §13 performance gate: the store must sustain **>= 2,000 exchanges/s**
//! committed through the REAL writer path at the §13 body mix.
//!
//! Exchanges go through the same `ExchangeSink::submit` the proxy uses: the
//! bounded queue, then the OS writer thread, which commits batches (default
//! config: `BEGIN IMMEDIATE`, commit every 250 ms / 1,000 exchanges). The
//! synthetic bulk insert is deliberately not used.
//!
//! The rate counts COMMITTED exchanges only. `Event::Committed` ids are
//! counted off a subscriber thread (also used as flow control, so the queue is
//! never overrun into the journal-spill path), and after the writer drains the
//! committed total is cross-checked against `count(*) ... state='committed'`
//! read from the database. rate = committed / wall time from the first submit
//! to the last commit (so it includes the final drain; never an overcount).
//!
//! Body mix (as `synthetic.rs`): 60 % < 4 KiB, 30 % 4-64 KiB, 9 % 64 KiB-1 MiB,
//! 1 % > 1 MiB (up to 8 MiB). Asserts `>= 2000/s` only under
//! `CROSSWIRE_PERF_ASSERT=1` (maintainer gate); otherwise report-only.
//!
//! Run with `--release`: debug zstd/SHA-256 on multi-MiB bodies is far slower.
//! The disk matters (every commit is fsynced); the authoritative figure is the
//! maintainer run on the reference machines.

use bytes::Bytes;
use crosswire_model::{
    Body, ConnectionInfo, Event, Exchange, Fidelity, FramingRecord, Head, Message, MessageKind,
};
use crosswire_store::{Project, Store, StoreConfig};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use ulid::Ulid;

const KIB: usize = 1024;
const MIB: usize = 1024 * 1024;
/// Submit window.
const WINDOW: Duration = Duration::from_secs(3);
/// Most exchanges submitted and not yet committed (the default queue holds
/// 10,000); with the byte bound this keeps the bounded queue from overflowing
/// into the journal-spill path.
const IN_FLIGHT: u64 = 9000;
/// Most queue bytes (bodies plus 1 KiB per exchange, as the sink charges) in
/// flight; the default queue holds 64 MiB, and past that `submit` spills to a
/// fsynced journal file, which is a different (slow) path than the one measured.
const IN_FLIGHT_BYTES: u64 = 56 * MIB as u64;
/// The §13 floor.
const MIN_RATE: f64 = 2000.0;

fn assert_enabled() -> bool {
    std::env::var("CROSSWIRE_PERF_ASSERT").as_deref() == Ok("1")
}

/// Body class 0..4 and its length, at the §13 proportions.
fn body_class_len(rng: &mut StdRng) -> (usize, usize) {
    match rng.random_range(0..100u32) {
        0..=59 => (0, rng.random_range(0..4 * KIB)),
        60..=89 => (1, rng.random_range(4 * KIB..64 * KIB)),
        90..=98 => (2, rng.random_range(64 * KIB..=MIB)),
        _ => (3, rng.random_range(MIB + 1..=8 * MIB)),
    }
}

/// Poorly compressible filler, unique per exchange (so nothing dedups).
fn fill(buf: &mut [u8], seed: u64) {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    for chunk in buf.chunks_mut(8) {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let b = x.to_le_bytes();
        chunk.copy_from_slice(&b[..chunk.len()]);
    }
}

fn msg(body: Vec<u8>) -> Message {
    Message::new(
        MessageKind::Normalized,
        Fidelity::Normalized,
        Head::Normalized { map: Vec::new() },
        FramingRecord::ContentLength {
            len: body.len() as u64,
        },
        Body::inline(Bytes::from(body)),
        ConnectionInfo::new("example.test", 443),
    )
}

#[test]
fn ingest_at_least_2k_per_sec() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = StoreConfig {
        // No lag: the counter thread must see every Committed event.
        event_capacity: 4_000_000,
        ..StoreConfig::default()
    };
    let store = Store::open(Project::create(&tmp.path().join("p")).unwrap(), cfg).unwrap();
    let sink = store.sink();

    // Committed counter: a subscriber thread totals Committed ids.
    let committed = Arc::new(AtomicU64::new(0));
    let inflight_bytes = Arc::new(AtomicU64::new(0));
    let sizes: Arc<Mutex<HashMap<Ulid, u64>>> = Arc::default();
    let errors = Arc::new(AtomicU64::new(0));
    let mut ev = store.subscribe();
    let counter = {
        let committed = Arc::clone(&committed);
        let inflight_bytes = Arc::clone(&inflight_bytes);
        let sizes = Arc::clone(&sizes);
        let errors = Arc::clone(&errors);
        std::thread::spawn(move || {
            while let Some(e) = ev.blocking_recv() {
                match e {
                    Event::Committed { ids, .. } => {
                        let mut sizes = sizes.lock().unwrap();
                        let freed: u64 = ids.iter().filter_map(|id| sizes.remove(id)).sum();
                        drop(sizes);
                        inflight_bytes.fetch_sub(freed, Ordering::SeqCst);
                        committed.fetch_add(ids.len() as u64, Ordering::SeqCst);
                    }
                    Event::Error { .. } | Event::Lagged { .. } => {
                        errors.fetch_add(1, Ordering::SeqCst);
                    }
                    _ => {}
                }
            }
        })
    };

    let mut rng = StdRng::seed_from_u64(13);
    let mut classes = [0u64; 4];
    let mut submitted: u64 = 0;
    let mut body_bytes: u64 = 0;
    let start = Instant::now();
    while start.elapsed() < WINDOW {
        if submitted - committed.load(Ordering::SeqCst) >= IN_FLIGHT
            || inflight_bytes.load(Ordering::SeqCst) >= IN_FLIGHT_BYTES
        {
            std::thread::sleep(Duration::from_micros(200));
            continue;
        }
        let (class, len) = body_class_len(&mut rng);
        classes[class] += 1;
        body_bytes += len as u64;
        let mut resp = vec![0u8; len];
        fill(&mut resp, submitted);
        let mut req = vec![0u8; 64];
        fill(&mut req, !submitted);
        let charge = (1024 + 64 + len) as u64;
        let mut ex = Exchange::new(msg(req));
        ex.set_response(msg(resp));
        sizes.lock().unwrap().insert(ex.id(), charge);
        inflight_bytes.fetch_add(charge, Ordering::SeqCst);
        sink.submit(ex).expect("submit through the real sink");
        submitted += 1;
    }
    let submit_secs = start.elapsed().as_secs_f64();

    // Wait for the writer to commit everything submitted.
    let deadline = Instant::now() + Duration::from_secs(120);
    while committed.load(Ordering::SeqCst) < submitted && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(2));
    }
    let secs = start.elapsed().as_secs_f64();
    let by_events = committed.load(Ordering::SeqCst);

    // Durable cross-check straight from the database.
    let by_db: u64 = store
        .reader()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM exchanges WHERE state = 'committed'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    // The sink shares the event sender; drop it so the counter thread sees the stream end.
    drop(sink);
    store.close().unwrap();
    counter.join().unwrap();

    assert_eq!(
        errors.load(Ordering::SeqCst),
        0,
        "error/lag events: queue overran"
    );
    assert_eq!(by_events, submitted, "writer did not commit everything");
    assert_eq!(by_db, submitted, "database committed count differs");

    let rate = by_db as f64 / secs;
    let pct = |c: u64| 100.0 * c as f64 / submitted as f64;
    println!(
        "SUMMARY ingest committed={by_db} secs={secs:.2} rate={rate:.0} \
         mb_per_s={:.0} submit_secs={submit_secs:.2} mix={:.1}/{:.1}/{:.1}/{:.1}",
        body_bytes as f64 / MIB as f64 / secs,
        pct(classes[0]),
        pct(classes[1]),
        pct(classes[2]),
        pct(classes[3])
    );

    // The mix measured is the §13 one, not uniform (tolerances for ~6k draws;
    // the 1 % class is rare, so it is only bounded).
    assert!(submitted >= 500, "too few exchanges for a mix check");
    assert!((pct(classes[0]) - 60.0).abs() < 6.0, "<4KiB share off");
    assert!((pct(classes[1]) - 30.0).abs() < 6.0, "4-64KiB share off");
    assert!((pct(classes[2]) - 9.0).abs() < 4.0, "64KiB-1MiB share off");
    assert!(pct(classes[3]) < 4.0, ">1MiB share off");

    if assert_enabled() {
        assert!(
            rate >= MIN_RATE,
            "ingest {rate:.0}/s below the {MIN_RATE}/s floor"
        );
    }
}
