//! The §13 named history queries over a synthetic project.
//!
//! Loads `generate(store, N, SEED)` (`CROSSWIRE_BENCH_N`, default 100 000; the
//! full gate is 1 000 000), then times every named query through
//! `Store::list`:
//!
//! - warm: repeated on the pooled, already-open reader connection;
//! - cold: after dropping the pooled connections, so each run opens a fresh
//!   connection with an empty SQLite page cache (the OS page cache is not
//!   dropped; that needs root and is not portable).
//!
//! It records the peak anonymous RSS ([`rss::anon_rss_bytes`], never `VmRSS`)
//! across the load, every query and a scroll of the last 200 rows over
//! several pages, then prints a `SUMMARY` block for `just perf` to compare
//! with `bench/README.md`. `CROSSWIRE_BENCH_QUICK=1` cuts the runs per query
//! (and skips criterion's own sampling) for the 1 000 000 gate. It reports; the 50 ms / 500 ms / 500 MB thresholds
//! are asserted by the gate, not here.
#![allow(missing_docs)]

#[path = "../src/rss.rs"]
mod rss;

use std::time::{Duration, Instant};

use criterion::{criterion_group, criterion_main, Criterion};
use crosswire_model::ScopeState;
use crosswire_store::synthetic::{generate, ADMIN_SEGMENT, HOST_X, TAG_Y};
use crosswire_store::{ExchangeFilter, Project, Store, StoreConfig};

const SEED: u64 = 7;
const DEFAULT_N: usize = 100_000;
const WARM_RUNS: usize = 200;
const COLD_RUNS: usize = 20;
/// `CROSSWIRE_BENCH_QUICK=1`: few runs per query and no criterion sampling,
/// for sizes (1 000 000) where a slow query would otherwise take an hour.
const QUICK_WARM_RUNS: usize = 8;
const QUICK_COLD_RUNS: usize = 4;
const SCROLL_PAGES: usize = 10;

/// Whether the query is a structured (indexed) one or a bounded path scan.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Structured,
    PathScan,
}

fn queries() -> Vec<(&'static str, Kind, ExchangeFilter)> {
    let f = ExchangeFilter::default;
    vec![
        (
            "host",
            Kind::Structured,
            ExchangeFilter {
                host: Some(HOST_X.into()),
                ..f()
            },
        ),
        (
            "status>=500 & POST",
            Kind::Structured,
            ExchangeFilter {
                status: Some(500..=599),
                method: Some("POST".into()),
                ..f()
            },
        ),
        (
            "min_resp_len=1048577",
            Kind::Structured,
            ExchangeFilter {
                min_resp_len: Some(1_048_577),
                ..f()
            },
        ),
        (
            "tag=Y",
            Kind::Structured,
            ExchangeFilter {
                tag: Some(TAG_Y.into()),
                ..f()
            },
        ),
        (
            "in-scope",
            Kind::Structured,
            ExchangeFilter {
                scope_state: Some(ScopeState::In),
                ..f()
            },
        ),
        (
            "path_contains",
            Kind::PathScan,
            ExchangeFilter {
                path_contains: Some(ADMIN_SEGMENT.into()),
                ..f()
            },
        ),
        (
            "path_regex",
            Kind::PathScan,
            ExchangeFilter {
                path_regex: Some(format!("^{ADMIN_SEGMENT}panel/[0-9]+$")),
                ..f()
            },
        ),
    ]
}

/// Tracks the highest anonymous RSS seen.
struct Peak(u64);

impl Peak {
    fn sample(&mut self) {
        if let Some(b) = rss::anon_rss_bytes() {
            self.0 = self.0.max(b);
        }
    }
}

fn p95(samples: &mut [Duration]) -> Duration {
    samples.sort();
    samples[(samples.len() * 95).div_ceil(100).saturating_sub(1)]
}

fn run_once(store: &Store, filter: &ExchangeFilter) -> bool {
    let page = store.list(filter, None, 200).expect("list");
    page.capped
}

fn bench_history(c: &mut Criterion) {
    let n: usize = std::env::var("CROSSWIRE_BENCH_N")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_N);
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::open(
        Project::create(dir.path()).expect("project"),
        StoreConfig::default(),
    )
    .expect("store");

    let quick = std::env::var_os("CROSSWIRE_BENCH_QUICK").is_some();
    let (warm_runs, cold_runs) = if quick {
        (QUICK_WARM_RUNS, QUICK_COLD_RUNS)
    } else {
        (WARM_RUNS, COLD_RUNS)
    };
    let mut peak = Peak(0);
    let rss_start = rss::anon_rss_bytes();
    peak.sample();
    let t = Instant::now();
    generate(&store, n, SEED);
    let load = t.elapsed();
    peak.sample();

    let mut group = c.benchmark_group("history_warm");
    group.sample_size(10);
    let mut rows = Vec::new();
    for (name, kind, filter) in queries() {
        run_once(&store, &filter); // warm the connection and the cache
        if !quick {
            group.bench_function(name, |b| b.iter(|| run_once(&store, &filter)));
        }
        let mut warm = Vec::with_capacity(warm_runs);
        let mut capped = false;
        for _ in 0..warm_runs {
            let t = Instant::now();
            capped |= run_once(&store, &filter);
            warm.push(t.elapsed());
            peak.sample();
        }
        let mut cold = Vec::with_capacity(cold_runs);
        for _ in 0..cold_runs {
            store.drop_idle_readers();
            let t = Instant::now();
            run_once(&store, &filter);
            cold.push(t.elapsed());
            peak.sample();
        }
        rows.push((name, kind, p95(&mut warm), p95(&mut cold), capped));
    }
    group.finish();

    // Scroll: the newest 200 rows, then several more pages by cursor.
    let mut after = None;
    let t = Instant::now();
    let mut paged = 0;
    for _ in 0..SCROLL_PAGES {
        let page = store
            .list(&ExchangeFilter::default(), after, 200)
            .expect("scroll");
        paged += page.items.len();
        peak.sample();
        match page.next_cursor {
            Some(cur) => after = Some(cur),
            None => break,
        }
    }
    let scroll = t.elapsed();

    println!("SUMMARY n={n} seed={SEED} load={load:.2?}");
    for (name, kind, warm, cold, capped) in &rows {
        let tag = match kind {
            Kind::Structured => "structured",
            Kind::PathScan => "path-scan",
        };
        println!(
            "SUMMARY query={name} kind={tag} warm_p95_ms={:.3} cold_p95_ms={:.3} capped={capped}",
            warm.as_secs_f64() * 1e3,
            cold.as_secs_f64() * 1e3,
        );
    }
    println!(
        "SUMMARY scroll pages={SCROLL_PAGES} rows={paged} total_ms={:.3}",
        scroll.as_secs_f64() * 1e3
    );
    println!(
        "SUMMARY anon_rss_start_mb={:.1} anon_rss_peak_mb={:.1}",
        rss_start.map_or(f64::NAN, |b| b as f64 / 1_048_576.0),
        peak.0 as f64 / 1_048_576.0,
    );
}

criterion_group! {
    name = benches;
    config = Criterion::default().warm_up_time(Duration::from_millis(200)).measurement_time(Duration::from_secs(1));
    targets = bench_history
}
criterion_main!(benches);
