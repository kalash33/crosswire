//! Selftest for the shared flat-RSS harness (`flat_rss/mod.rs`).
//!
//! Proves the harness genuinely catches anonymous-RSS growth: on a host where
//! `rss_anon_kib()` reports a figure (Linux, the CI gate), driving a 64 MiB
//! allocation under a 4 MiB cap must FAIL the harness assertion, while a
//! no-allocation drive under a 64 MiB cap must pass. On a host where the probe
//! is unavailable (macOS/other) the harness takes its SKIP path and neither
//! drive asserts.

use std::time::Duration;

#[path = "flat_rss/mod.rs"]
mod flat_rss;

/// Runs `with_flat_rss` to completion on its own thread + runtime so an
/// intentional harness panic is observed as a thread-join error rather than
/// unwinding the test itself. `alloc` controls whether the drive allocates and
/// touches 64 MiB and holds it across a sampling window.
fn run_with_flat_rss(cap_mib: u64, alloc: bool) -> std::thread::Result<()> {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("runtime builds");
        rt.block_on(async move {
            flat_rss::with_flat_rss(cap_mib, || async move {
                if alloc {
                    let mut v: Vec<u8> = vec![0u8; 64 * 1024 * 1024];
                    // Touch a byte per page so the pages are actually resident.
                    let mut i = 0;
                    while i < v.len() {
                        v[i] = (i & 0xff) as u8;
                        i += 4096;
                    }
                    // Hold across several sampler ticks (~10ms each).
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    std::hint::black_box(&v);
                    drop(v);
                } else {
                    tokio::time::sleep(Duration::from_millis(30)).await;
                }
            })
            .await;
        });
    })
    .join()
}

#[test]
fn flat_rss_harness_catches_growth() {
    // The 64 MiB case panics on purpose; silence the default hook's backtrace
    // so the intentional failure does not look like a real one in the log.
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let grow = run_with_flat_rss(4, true);
    let flat = run_with_flat_rss(64, false);
    std::panic::set_hook(prev);

    if flat_rss::rss_anon_kib().is_some() {
        // Linux / CI gate: the harness must catch the 64 MiB growth under the
        // 4 MiB cap, and must pass the no-allocation drive under the 64 MiB cap.
        assert!(
            grow.is_err(),
            "flat-RSS harness failed to catch 64 MiB growth under a 4 MiB cap"
        );
        assert!(
            flat.is_ok(),
            "no-allocation drive under a 64 MiB cap must pass"
        );
    } else {
        // macOS / other: the probe is unavailable, so the harness SKIPs its
        // assertion and both drives complete without panicking.
        assert!(grow.is_ok(), "SKIP path must not assert (64 MiB drive)");
        assert!(flat.is_ok(), "SKIP path must not assert (no-alloc drive)");
    }
}
