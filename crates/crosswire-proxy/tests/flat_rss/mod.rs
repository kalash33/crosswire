//! Shared flat-RSS harness for the §11.6 Limits rows that §13 gate #18
//! requires to hold anonymous RSS flat under attack.
//!
//! Each flat-RSS test binary includes this module with
//! `#[path = "flat_rss/mod.rs"] mod flat_rss;` (mirroring how the proxy tests
//! share `common/mod.rs`). It provides:
//!
//! * [`rss_anon_kib`] — the process's anonymous resident memory in KiB. On
//!   Linux it parses the `RssAnon:` line of `/proc/self/status` (NOT `VmRSS`:
//!   `RssAnon` excludes the file-backed, mmap'd pages that are irrelevant to an
//!   allocation bound). Elsewhere it returns `None`, which makes the assertion
//!   SKIP — CI runs on Linux, which is the gate. The store keeps the canonical
//!   probe in `crosswire-store/src/rss.rs`; this is a std-only copy because
//!   `crosswire-store` is not a proxy test dependency for this metric (just as
//!   `limits_rss.rs` reproduces its own RSS probe).
//! * [`with_flat_rss`] — acquires a process-global serialization guard (RSS is
//!   process-wide, so these tests must not run concurrently with other
//!   allocating tests), samples a baseline, runs a background sampler that
//!   records the peak while the hostile input is driven to its cap, then
//!   asserts the growth stayed under `cap_mib`.

#![allow(dead_code)]

use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Process-global guard: RSS is a property of the whole process, so only one
/// flat-RSS measurement may be in flight at a time. Held for the entire
/// measured section (baseline through assertion). A poisoned lock (left by an
/// earlier intentional-failure test) is recovered so one failure does not
/// cascade.
static GUARD: Mutex<()> = Mutex::new(());

/// The process's anonymous resident memory in KiB, excluding file-backed
/// (mmap'd) pages.
///
/// Linux: the `RssAnon:` line of `/proc/self/status`, already in KiB.
/// Other targets: `None`, which makes [`with_flat_rss`] SKIP its assertion.
#[cfg(target_os = "linux")]
#[must_use]
pub fn rss_anon_kib() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let rest = status
        .lines()
        .find_map(|l| l.strip_prefix("RssAnon:"))?
        .trim();
    rest.strip_suffix("kB").unwrap_or(rest).trim().parse().ok()
}

/// No anonymous-RSS probe on this target; the harness SKIPs its assertion.
///
/// macOS has no cheap `RssAnon` equivalent without an extra crate, and the
/// `ps`/`VmRSS`-style figures include file-backed pages, so they are the wrong
/// metric for an allocation bound. CI runs on Linux, which is the gate.
#[cfg(not(target_os = "linux"))]
#[must_use]
pub fn rss_anon_kib() -> Option<u64> {
    None
}

/// Drives `drive` to its cap while asserting the process's anonymous RSS does
/// not grow by `cap_mib` MiB or more.
///
/// Holds [`GUARD`] for the whole measured section so no other allocating test
/// perturbs the reading. Samples a baseline, spawns a background sampler that
/// records the peak (via [`AtomicU64::fetch_max`]) every ~10ms while `drive`
/// runs, then asserts `peak - baseline < cap_mib * 1024` (KiB). On a host where
/// [`rss_anon_kib`] is `None` it prints a SKIP notice and runs `drive` without
/// asserting.
///
/// # Panics
///
/// Panics (failing the test) when the measured anonymous-RSS growth reaches or
/// exceeds `cap_mib` MiB. The message prints baseline, peak and growth.
// The guard is deliberately held across the `drive().await` (the measured
// region): that is what serializes flat-RSS tests against every other
// allocating task, which is the whole point of the probe. A `tokio::sync::Mutex`
// would drop the poison semantics this harness relies on (`into_inner` recovery
// below), so a std `Mutex` held across the await is the intended design.
#[allow(clippy::await_holding_lock)]
pub async fn with_flat_rss<F, Fut>(cap_mib: u64, drive: F)
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = ()>,
{
    // Recover a poisoned guard: an earlier flat-RSS test that panicked by
    // design must not poison every later one.
    let _guard = GUARD.lock().unwrap_or_else(|e| e.into_inner());

    let Some(baseline) = rss_anon_kib() else {
        eprintln!(
            "flat-RSS: rss_anon_kib() unavailable on this host; \
             SKIP (running drive without asserting)"
        );
        drive().await;
        return;
    };

    let peak = Arc::new(AtomicU64::new(baseline));
    let stop = Arc::new(AtomicBool::new(false));
    let sampler = {
        let peak = Arc::clone(&peak);
        let stop = Arc::clone(&stop);
        tokio::spawn(async move {
            while !stop.load(Ordering::Relaxed) {
                if let Ok(Some(kib)) = tokio::task::spawn_blocking(rss_anon_kib).await {
                    peak.fetch_max(kib, Ordering::Relaxed);
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
    };

    drive().await;

    stop.store(true, Ordering::Relaxed);
    sampler.abort();
    let _ = sampler.await;

    let peak = peak.load(Ordering::Relaxed);
    let growth = peak.saturating_sub(baseline);
    let cap_kib = cap_mib.saturating_mul(1024);
    assert!(
        growth < cap_kib,
        "flat-RSS exceeded: baseline {baseline} KiB, peak {peak} KiB, \
         growth {growth} KiB >= cap {cap_kib} KiB ({cap_mib} MiB)"
    );
}
