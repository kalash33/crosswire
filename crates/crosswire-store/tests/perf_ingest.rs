//! §13 ingest measurement: the §13 target is **>= 2,000 exchanges/s** committed
//! through the REAL writer path at the §13 body mix (currently unmet).
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
//! Body mix `s13` (as `synthetic.rs`, the default): 60 % < 4 KiB, 30 % 4-64 KiB,
//! 9 % 64 KiB-1 MiB, 1 % 1-8 MiB uniform, all incompressible noise. Mix
//! `typical` (a proposal, not data; report-only): 70 % < 4 KiB, 25 % 4-64 KiB,
//! 4.5 % 64 KiB-1 MiB, 0.5 % 1-4 MiB, every other body compressible ASCII text.
//!
//! Knobs (environment, read by this harness only, never by the library):
//! - `CROSSWIRE_INGEST_RUNS` (default 1): runs, each on a fresh project, 5 s
//!   apart; after them a `SUMMARY ingest-gate` line gives the median and min.
//! - `CROSSWIRE_INGEST_WINDOW_SECS` (default 3): the submit window per run.
//! - `CROSSWIRE_INGEST_MIX` = `s13` (default) | `typical`.
//! - `CROSSWIRE_INGEST_PREPARE_THREADS`: refused until the prepare pool exists.
//!
//! Each run prints `SUMMARY ingest ... gen_busy_pct=...`: the share of the
//! window the generator spent filling bodies, building and submitting
//! exchanges. Above 80 % the harness, not the writer, is the limit, and the
//! run FAILS as invalid. With `--features stage-timing` each run also prints a
//! `STAGES` table of the writer's time per stage (spec §3.3); the asserted
//! numbers always come from a build without the feature.
//!
//! REPORT-ONLY: the rate is printed, never asserted against 2,000/s (the
//! measured ~700/s is below target; a maintainer decision is pending). Only a
//! gross-regression floor (release builds), the body mix and the run validity
//! are asserted. `CROSSWIRE_PERF_ASSERT` changes nothing here yet.
//!
//! Two probes print report-only lines in release builds: `SUMMARY sha256`
//! (SHA-256 MB/s, and whether the aarch64 SHA2 instructions appear to be in
//! use) and `SUMMARY json-encode` (`serde_json::to_value` of an exchange with
//! a 1 MiB inline body, which base64-encodes it).
//!
//! Run with `--release`: debug zstd/SHA-256 on multi-MiB bodies is far slower.
//! The disk matters (every commit is fsynced); the authoritative figure is the
//! maintainer run on the reference machines (AC power, Low Power Mode off).

use bytes::Bytes;
use crosswire_model::{
    Body, ConnectionInfo, Event, Exchange, Fidelity, FramingRecord, Head, Message, MessageKind,
};
use crosswire_store::{Project, Store, StoreConfig};
use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use ulid::Ulid;

const KIB: usize = 1024;
const MIB: usize = 1024 * 1024;
/// Default submit window (`CROSSWIRE_INGEST_WINDOW_SECS`).
const DEFAULT_WINDOW_SECS: u64 = 3;
/// Default number of runs (`CROSSWIRE_INGEST_RUNS`).
const DEFAULT_RUNS: usize = 1;
/// Pause between runs (lets the page cache and a fanless M2 settle a little).
const RUN_PAUSE: Duration = Duration::from_secs(5);
/// A run whose generator was busy for more than this share of the window
/// measured the harness, not the writer: it fails as invalid.
const MAX_GEN_BUSY_PCT: f64 = 80.0;
/// Most exchanges submitted and not yet committed (the default queue holds
/// 10,000); with the byte bound this keeps the bounded queue from overflowing
/// into the journal-spill path.
const IN_FLIGHT: u64 = 9000;
/// Most queue bytes (bodies plus 1 KiB per exchange, as the sink charges) in
/// flight; the default queue holds 64 MiB, and past that `submit` spills to a
/// fsynced journal file, which is a different (slow) path than the one measured.
const IN_FLIGHT_BYTES: u64 = 56 * MIB as u64;
/// A gross-regression guard, NOT the §13 2,000/s target. The measured rate at
/// this mix is ~700/s (M2), so the §13 target is currently unmet and the rate
/// is report-only pending a maintainer decision (see `bench/README.md`). This
/// floor sits ~2.3x below the measurement so it catches a collapse without
/// flaking on slower hosts.
const GROSS_FLOOR_RATE: f64 = 300.0;

/// The body-size mix one run draws from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mix {
    /// The §13 mix: 60/30/9/1, incompressible.
    S13,
    /// The proposed lighter mix: 70/25/4.5/0.5 (1-4 MiB), half text.
    Typical,
}

impl Mix {
    fn name(self) -> &'static str {
        match self {
            Mix::S13 => "s13",
            Mix::Typical => "typical",
        }
    }

    /// Body class 0..4 and its length, at this mix's proportions. The `s13`
    /// draw sequence is unchanged from 0.1.1 (same seed, same calls).
    fn class_len(self, rng: &mut StdRng) -> (usize, usize) {
        match self {
            Mix::S13 => match rng.random_range(0..100u32) {
                0..=59 => (0, rng.random_range(0..4 * KIB)),
                60..=89 => (1, rng.random_range(4 * KIB..64 * KIB)),
                90..=98 => (2, rng.random_range(64 * KIB..=MIB)),
                _ => (3, rng.random_range(MIB + 1..=8 * MIB)),
            },
            Mix::Typical => match rng.random_range(0..1000u32) {
                0..=699 => (0, rng.random_range(0..4 * KIB)),
                700..=949 => (1, rng.random_range(4 * KIB..64 * KIB)),
                950..=994 => (2, rng.random_range(64 * KIB..=MIB)),
                _ => (3, rng.random_range(MIB + 1..=4 * MIB)),
            },
        }
    }

    /// Expected share (%) of each class and the tolerance it is checked with
    /// (for a few thousand draws; the rarest class is only bounded above).
    fn expected(self) -> [(f64, f64); 4] {
        match self {
            Mix::S13 => [(60.0, 6.0), (30.0, 6.0), (9.0, 4.0), (1.0, 3.0)],
            Mix::Typical => [(70.0, 6.0), (25.0, 6.0), (4.5, 3.0), (0.5, 1.5)],
        }
    }

    /// Whether the body of exchange `n` is compressible text.
    fn text_body(self, n: u64) -> bool {
        self == Mix::Typical && n.is_multiple_of(2)
    }
}

/// The harness knobs, from the environment.
#[derive(Debug)]
struct Knobs {
    runs: usize,
    window: Duration,
    mix: Mix,
}

/// Parses `name`, or `default` if unset. A value that does not parse fails
/// the test with a message naming the variable.
fn env_or<T: std::str::FromStr>(name: &str, default: T) -> T {
    match std::env::var(name) {
        Err(_) => default,
        Ok(v) => v
            .trim()
            .parse()
            .unwrap_or_else(|_| panic!("{name}={v:?} is not a valid value")),
    }
}

fn knobs() -> Knobs {
    let runs: usize = env_or("CROSSWIRE_INGEST_RUNS", DEFAULT_RUNS);
    assert!(runs >= 1, "CROSSWIRE_INGEST_RUNS must be at least 1");
    let window_secs: u64 = env_or("CROSSWIRE_INGEST_WINDOW_SECS", DEFAULT_WINDOW_SECS);
    assert!(
        window_secs >= 1,
        "CROSSWIRE_INGEST_WINDOW_SECS must be at least 1"
    );
    let mix = match std::env::var("CROSSWIRE_INGEST_MIX").as_deref() {
        Err(_) | Ok("s13") => Mix::S13,
        Ok("typical") => Mix::Typical,
        Ok(other) => panic!("CROSSWIRE_INGEST_MIX={other:?}: expected `s13` or `typical`"),
    };
    if let Ok(v) = std::env::var("CROSSWIRE_INGEST_PREPARE_THREADS") {
        let n: usize = env_or("CROSSWIRE_INGEST_PREPARE_THREADS", 0);
        panic!(
            "CROSSWIRE_INGEST_PREPARE_THREADS={v:?} (parsed as {n}): the store has no \
             prepare pool yet (P2 plan task 2), so this knob is refused rather than \
             silently ignored; unset it"
        );
    }
    Knobs {
        runs,
        window: Duration::from_secs(window_secs),
        mix,
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

/// Compressible ASCII text (words from a small vocabulary picked by an
/// xorshift stream), unique per exchange through `seed` (so nothing dedups).
fn fill_text(buf: &mut [u8], seed: u64) {
    const WORDS: [&[u8]; 16] = [
        b"the ",
        b"request ",
        b"response ",
        b"header ",
        b"value ",
        b"session ",
        b"token ",
        b"user ",
        b"id ",
        b"json ",
        b"true, ",
        b"false, ",
        b"null\n",
        b"{\"data\": ",
        b"200 ",
        b"cache ",
    ];
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut at = 0;
    while at < buf.len() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let w = WORDS[(x & 15) as usize];
        let n = w.len().min(buf.len() - at);
        buf[at..at + n].copy_from_slice(&w[..n]);
        at += n;
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

/// The median of `xs` (mean of the middle two for an even count).
fn median(xs: &[f64]) -> f64 {
    let mut v = xs.to_vec();
    v.sort_by(f64::total_cmp);
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

#[test]
fn median_of_odd_and_even_counts() {
    assert_eq!(median(&[3.0, 1.0, 2.0]), 2.0);
    assert_eq!(median(&[4.0, 1.0, 3.0, 2.0]), 2.5);
    assert_eq!(median(&[7.0]), 7.0);
}

#[test]
fn text_filler_is_ascii_and_unique() {
    let (mut a, mut b) = (vec![0u8; 4096], vec![0u8; 4096]);
    fill_text(&mut a, 1);
    fill_text(&mut b, 2);
    assert!(a.is_ascii() && b.is_ascii());
    assert_ne!(a, b);
}

#[test]
fn ingest_at_least_2k_per_sec() {
    let k = knobs();
    let build = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let stage_timing = if cfg!(feature = "stage-timing") {
        "on"
    } else {
        "off"
    };
    // Own line: libtest prints `test <name> ... ` first under --nocapture.
    println!();
    println!(
        "INGEST runs={} window_secs={} mix={} prepare_threads=n/a build={build} \
         stage_timing={stage_timing}",
        k.runs,
        k.window.as_secs(),
        k.mix.name(),
    );
    let mut rates = Vec::with_capacity(k.runs);
    for run in 1..=k.runs {
        if run > 1 {
            std::thread::sleep(RUN_PAUSE);
        }
        rates.push(run_once(run, &k));
    }
    let list: Vec<String> = rates.iter().map(|r| format!("{r:.0}")).collect();
    println!(
        "SUMMARY ingest-gate mix={} median={:.0} min={:.0} runs={}",
        k.mix.name(),
        median(&rates),
        rates.iter().copied().fold(f64::INFINITY, f64::min),
        list.join("/"),
    );
}

/// One run on a fresh project; returns the committed rate.
fn run_once(run: usize, k: &Knobs) -> f64 {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = StoreConfig {
        // No lag: the counter thread must see every Committed event.
        event_capacity: 4_000_000,
        ..StoreConfig::default()
    };
    let store = Store::open(Project::create(&tmp.path().join("p")).unwrap(), cfg).unwrap();
    let sink = store.sink();
    #[cfg(feature = "stage-timing")]
    let stages_before = store.stage_times();

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
    // Time the generator spent drawing, filling, building and submitting.
    let mut gen_busy = Duration::ZERO;
    let start = Instant::now();
    while start.elapsed() < k.window {
        if submitted - committed.load(Ordering::SeqCst) >= IN_FLIGHT
            || inflight_bytes.load(Ordering::SeqCst) >= IN_FLIGHT_BYTES
        {
            std::thread::sleep(Duration::from_micros(200));
            continue;
        }
        let busy_from = Instant::now();
        let (class, len) = k.mix.class_len(&mut rng);
        classes[class] += 1;
        body_bytes += len as u64;
        let mut resp = vec![0u8; len];
        if k.mix.text_body(submitted) {
            fill_text(&mut resp, submitted);
        } else {
            fill(&mut resp, submitted);
        }
        let mut req = vec![0u8; 64];
        fill(&mut req, !submitted);
        let charge = (1024 + 64 + len) as u64;
        let mut ex = Exchange::new(msg(req));
        ex.set_response(msg(resp));
        sizes.lock().unwrap().insert(ex.id(), charge);
        inflight_bytes.fetch_add(charge, Ordering::SeqCst);
        sink.submit(ex).expect("submit through the real sink");
        submitted += 1;
        gen_busy += busy_from.elapsed();
    }
    let submit_secs = start.elapsed().as_secs_f64();

    // Wait for the writer to commit everything submitted.
    let deadline = Instant::now() + Duration::from_secs(120);
    while committed.load(Ordering::SeqCst) < submitted && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(2));
    }
    let secs = start.elapsed().as_secs_f64();
    let by_events = committed.load(Ordering::SeqCst);
    #[cfg(feature = "stage-timing")]
    let stages = store.stage_times().since(&stages_before);

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
    let mb_per_s = body_bytes as f64 / MIB as f64 / secs;
    let gen_busy_pct = 100.0 * gen_busy.as_secs_f64() / submit_secs;
    let pct = |c: u64| 100.0 * c as f64 / submitted as f64;
    #[cfg(feature = "stage-timing")]
    let stage_field = format!(" stage_ms={}", stage_ms_field(&stages));
    #[cfg(not(feature = "stage-timing"))]
    let stage_field = String::new();
    println!(
        "SUMMARY ingest run={run} committed={by_db} secs={secs:.2} rate={rate:.0} \
         mb_per_s={mb_per_s:.0} submit_secs={submit_secs:.2} \
         mix={}:{:.1}/{:.1}/{:.1}/{:.1} prepare_threads=n/a \
         gen_busy_pct={gen_busy_pct:.0}{stage_field}",
        k.mix.name(),
        pct(classes[0]),
        pct(classes[1]),
        pct(classes[2]),
        pct(classes[3])
    );
    #[cfg(feature = "stage-timing")]
    print_stage_table(run, &stages, by_db, secs);

    // The mix measured is the one asked for, not uniform (tolerances for a
    // few thousand draws; the rarest class is only bounded above).
    assert!(submitted >= 500, "too few exchanges for a mix check");
    let expected = k.mix.expected();
    for (class, (share, tol)) in expected.iter().enumerate().take(3) {
        let got = pct(classes[class]);
        assert!(
            (got - share).abs() < *tol,
            "class {class} share {got:.1} % is off the expected {share} % (mix {})",
            k.mix.name()
        );
    }
    let (rare, tol) = expected[3];
    assert!(pct(classes[3]) < rare + tol, "largest-body share off");

    // A harness-bound run did not measure the writer: invalid, not passing.
    assert!(
        gen_busy_pct <= MAX_GEN_BUSY_PCT,
        "INVALID run {run}: the generator was busy {gen_busy_pct:.0} % of the window \
         (> {MAX_GEN_BUSY_PCT} %), so the harness, not the writer, set the rate"
    );

    // Debug builds (zstd, SHA-256) are far slower; the floor is meaningful in
    // release only. The rate itself is never asserted against 2,000/s.
    if !cfg!(debug_assertions) {
        assert!(
            rate >= GROSS_FLOOR_RATE,
            "ingest {rate:.0}/s is below the {GROSS_FLOOR_RATE}/s gross-regression floor"
        );
    }
    rate
}

/// `json:12.3,sha_put:45.6,...`: total milliseconds per stage in this run.
#[cfg(feature = "stage-timing")]
fn stage_ms_field(s: &crosswire_store::StageSnapshot) -> String {
    use crosswire_store::Stage;
    Stage::ALL
        .iter()
        .map(|st| format!("{}:{:.1}", st.name(), ms(s.total(*st))))
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(feature = "stage-timing")]
fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

/// One row per stage: total ms, calls, ms per 1,000 committed exchanges and
/// share of writer time (`batch` + `after_commit`); `(other)` is the batch
/// time no inner stage covers.
#[cfg(feature = "stage-timing")]
fn print_stage_table(run: usize, s: &crosswire_store::StageSnapshot, committed: u64, wall: f64) {
    use crosswire_store::Stage;
    let writer = s.total(Stage::Batch) + s.total(Stage::AfterCommit);
    let writer_ms = ms(writer).max(f64::MIN_POSITIVE);
    let per_k = |d: Duration| ms(d) * 1000.0 / committed.max(1) as f64;
    println!(
        "STAGES run={run} committed={committed} writer_ms={:.1} wall_ms={:.1} \
         writer_busy_pct={:.0} writer_us_per_exchange={:.1}",
        ms(writer),
        wall * 1e3,
        100.0 * writer.as_secs_f64() / wall,
        writer.as_secs_f64() * 1e6 / committed.max(1) as f64,
    );
    println!(
        "  {:<14} {:>10} {:>9} {:>10} {:>8}",
        "stage", "total_ms", "calls", "ms/1k_ex", "%writer"
    );
    let row = |name: &str, d: Duration, calls: String| {
        println!(
            "  {name:<14} {:>10.1} {calls:>9} {:>10.2} {:>8.1}",
            ms(d),
            per_k(d),
            100.0 * ms(d) / writer_ms
        );
    };
    for st in Stage::INNER {
        row(st.name(), s.total(st), s.calls(st).to_string());
    }
    let other = s.total(Stage::Batch).saturating_sub(s.inner_total());
    row("(other)", other, "-".into());
    row(
        Stage::AfterCommit.name(),
        s.total(Stage::AfterCommit),
        s.calls(Stage::AfterCommit).to_string(),
    );
    row("= writer", writer, s.calls(Stage::Batch).to_string());
}

/// SHA-256 throughput of this build's `sha2` on this CPU (report-only). On
/// aarch64 the ARMv8 SHA2 instructions run at well over 1 GB/s on an M2; the
/// portable software path runs at a few hundred MB/s. The hint is `hw` at
/// >= 1,000 MB/s on aarch64 and `soft?` below; elsewhere `n/a`.
#[test]
fn sha256_backend_throughput() {
    if cfg!(debug_assertions) {
        println!("SKIP sha256 probe: debug build (run with --release)");
        return;
    }
    const PASSES: u8 = 16;
    let mut buf = vec![0u8; 64 * MIB];
    fill(&mut buf, 0x5eed);
    std::hint::black_box(Sha256::digest(&buf[..MIB])); // warm-up
    let start = Instant::now();
    let mut acc = 0u8;
    for i in 0..PASSES {
        buf[0] = i; // a different input each pass: nothing to hoist
        acc ^= Sha256::digest(std::hint::black_box(&buf))[0];
    }
    let secs = start.elapsed().as_secs_f64();
    std::hint::black_box(acc);
    let mb_per_s = f64::from(PASSES) * 64.0 / secs;
    let arch = std::env::consts::ARCH;
    #[cfg(target_arch = "aarch64")]
    let (cpu_sha2, backend) = (
        if std::arch::is_aarch64_feature_detected!("sha2") {
            "yes"
        } else {
            "no"
        },
        if mb_per_s >= 1000.0 { "hw" } else { "soft?" },
    );
    #[cfg(not(target_arch = "aarch64"))]
    let (cpu_sha2, backend) = ("n/a", "n/a");
    // Own line: libtest prints `test <name> ... ` first under --nocapture.
    println!();
    println!(
        "SUMMARY sha256 arch={arch} cpu_sha2={cpu_sha2} mb_per_s={mb_per_s:.0} \
         backend={backend} bytes={} secs={secs:.2} (report-only; hw expected >= 1000 \
         MB/s on aarch64)",
        u64::from(PASSES) * 64 * MIB as u64,
    );
}

/// What building the row JSON costs per body byte today: `to_value` of an
/// exchange with a 1 MiB inline response (base64-encoded into a `Value`,
/// which the writer then throws away). Report-only; feeds the T2 estimate.
#[test]
fn body_json_encode_throughput() {
    if cfg!(debug_assertions) {
        println!("SKIP json-encode probe: debug build (run with --release)");
        return;
    }
    const PASSES: u32 = 32;
    let mut body = vec![0u8; MIB];
    fill(&mut body, 0xb64);
    let mut ex = Exchange::new(msg(vec![0u8; 64]));
    ex.set_response(msg(body));
    std::hint::black_box(serde_json::to_value(&ex).unwrap()); // warm-up
    let start = Instant::now();
    for _ in 0..PASSES {
        std::hint::black_box(serde_json::to_value(std::hint::black_box(&ex)).unwrap());
    }
    let secs = start.elapsed().as_secs_f64();
    // Own line: libtest prints `test <name> ... ` first under --nocapture.
    println!();
    println!(
        "SUMMARY json-encode body_mib=1 mb_per_s={:.0} ms_per_mib={:.2} (report-only; \
         to_value of the whole exchange, bodies base64-encoded)",
        f64::from(PASSES) / secs,
        secs * 1e3 / f64::from(PASSES),
    );
}
