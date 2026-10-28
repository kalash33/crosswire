//! Self-test for the in-process load harness (`load/mod.rs`): proves the
//! Direct driver measures real latency and throughput against the echo
//! upstream, and that the percentile reducer orders p50 ≤ p99. The §13
//! performance gates build on this harness; this binary guards it.

#[path = "load/mod.rs"]
mod load;

use std::time::Duration;

use load::{drive, percentiles, spawn_echo_upstream, Target};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn harness_measures_direct_latency_and_throughput() {
    let body_len = 16 * 1024;
    let upstream = spawn_echo_upstream(body_len).await;
    let stats = drive(
        Target::Direct(upstream),
        8,
        Duration::from_secs(1),
        body_len,
    )
    .await;

    assert!(stats.reqs > 0, "no requests completed");
    assert!(stats.p50_us > 0, "p50 must be positive, got {}", stats.p50_us);
    assert!(
        stats.p99_us >= stats.p50_us,
        "p99 ({}) must be >= p50 ({})",
        stats.p99_us,
        stats.p50_us
    );
    assert!(
        stats.reqs_per_sec() > 0.0,
        "throughput must be positive, got {}",
        stats.reqs_per_sec()
    );
}

#[test]
fn percentiles_are_ordered() {
    // A deliberately skewed synthetic latency set: the long tail must land
    // in p99, never below p50.
    let mut samples = vec![1u64, 2, 2, 3, 3, 3, 4, 5, 10, 1000];
    let (p50, p99) = percentiles(&mut samples);
    assert!(p50 > 0, "p50 must be positive");
    assert!(p50 <= p99, "p50 ({p50}) must be <= p99 ({p99})");
    // Nearest-rank on 10 samples: p50 → index ceil(0.50*10)-1 = 4 (value 3),
    // p99 → index ceil(0.99*10)-1 = 9 (value 1000).
    assert_eq!(p50, 3);
    assert_eq!(p99, 1000);
}
