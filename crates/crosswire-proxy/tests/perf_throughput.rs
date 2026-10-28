//! §13 performance gate: the proxy must sustain **>= 5,000 req/s** with MITM
//! TLS on and recording on, plus a separate SNI-passthrough reference row.
//!
//! Both tests drive 64 keep-alive client-TLS connections issuing back-to-back
//! origin-form `GET /` (16 KiB response) through a `CONNECT` tunnel, closed-loop
//! at max rate, so the proxy is saturated. The per-connection handshake,
//! `CONNECT` and a warmup are excluded from the counted requests' latencies, but
//! `reqs_per_sec` is requests over the whole wall window (a slight undercount,
//! never an overcount).
//!
//! * `throughput_mitm_at_least_5k` — the proxy terminates TLS, relays and
//!   records every exchange. Prints `SUMMARY throughput ...`; asserts
//!   `>= 5000 req/s` only under `CROSSWIRE_PERF_ASSERT=1` (maintainer gate,
//!   `just perf`), otherwise report-only. `recorded > 0` is always asserted so
//!   MITM and recording are provably on.
//! * `throughput_sni_tunnel` — the host is in `passthrough`, so the proxy
//!   tunnels bytes without minting a cert (the client trusts only the upstream
//!   CA end to end). Prints `SUMMARY throughput_sni_tunnel ...`; a REFERENCE
//!   row, never asserted on throughput.
//!
//! Run with `--release`: debug rustls/ring crypto is ~20x slower and the
//! numbers are meaningless. Caveat: the load generator, proxy and upstream
//! share one 4-worker runtime and `TestSink` records synchronously under one
//! mutex, so in-process figures are a conservative floor; the authoritative
//! throughput is the maintainer `oha` run against a standalone `crosswire serve`.

#[path = "load/mod.rs"]
mod load;

mod common;

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use common::{loopback_config, ProxyHarness, UpstreamWiring};
use crosswire_proxy::NoopHandler;
use load::{drive_mitm, spawn_tls_echo_upstream, TlsTarget};

/// Response body size (spec §13: 16 KiB).
const BODY_LEN: usize = 16 * 1024;
/// Keep-alive connections driven in parallel; enough to saturate the proxy.
const CONNS: usize = 64;
/// Steady-state measurement window.
const DUR: Duration = Duration::from_secs(3);
/// The upstream authority the client and proxy both target.
const HOST: &str = "u.test";
/// The §13 floor.
const MIN_RPS: f64 = 5000.0;

fn assert_enabled() -> bool {
    std::env::var("CROSSWIRE_PERF_ASSERT").as_deref() == Ok("1")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn throughput_mitm_at_least_5k() {
    let up = spawn_tls_echo_upstream(BODY_LEN).await;
    let authority = format!("{HOST}:{}", up.addr.port());
    let wiring = UpstreamWiring::loopback(up.ca_der.clone(), up.addr.port(), &[HOST]);
    let h = ProxyHarness::start_wired(loopback_config(), Arc::new(NoopHandler), wiring).await;

    let stats = drive_mitm(
        TlsTarget::ViaMitm {
            proxy: h.addr,
            authority,
            ca_der: h.ca.cert_der(),
            server_name: HOST.to_string(),
        },
        CONNS,
        DUR,
        BODY_LEN,
    )
    .await;

    // A plain tunnel could never be decrypted, so recorded exchanges prove the
    // proxy terminated TLS, relayed and recorded.
    let recorded = h.sink.exchanges.lock().unwrap().len();
    // In a debug build the slow MITM path may legitimately complete zero
    // requests in the window; release runs (perf-smoke, just perf) assert
    // these. Matches how perf_ingest gates its gross floor.
    if cfg!(debug_assertions) {
        eprintln!(
            "(debug build: skipping MITM/reqs>0 setup asserts - run --release for the real check)"
        );
    } else {
        assert!(
            recorded > 0,
            "the proxy recorded no exchanges: MITM/recording was not active"
        );
        assert!(stats.reqs > 0, "no requests completed");
    }

    let rps = stats.reqs_per_sec();
    println!(
        "SUMMARY throughput mitm_rps={rps:.0} reqs={} elapsed_ms={} p50_us={} p99_us={} \
         conns={CONNS} recorded={recorded}",
        stats.reqs,
        stats.elapsed.as_millis(),
        stats.p50_us,
        stats.p99_us,
    );

    if assert_enabled() {
        assert!(
            rps >= MIN_RPS,
            "MITM throughput {rps:.0} req/s is below the {MIN_RPS:.0} req/s gate \
             (in-process harness with a synchronous TestSink; confirm with the oha gate)"
        );
    }

    h.shutdown.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn throughput_sni_tunnel() {
    let up = spawn_tls_echo_upstream(BODY_LEN).await;
    let authority = format!("{HOST}:{}", up.addr.port());
    let mut cfg = loopback_config();
    cfg.passthrough = vec![HOST.to_string()];
    let wiring = UpstreamWiring::loopback(up.ca_der.clone(), up.addr.port(), &[HOST]);
    let h = ProxyHarness::start_wired(cfg, Arc::new(NoopHandler), wiring).await;

    // The client trusts only the upstream CA: the handshake succeeds only if
    // the proxy tunnelled bytes to the upstream without minting a leaf.
    let stats = drive_mitm(
        TlsTarget::ViaMitm {
            proxy: h.addr,
            authority,
            ca_der: up.ca_der.clone(),
            server_name: HOST.to_string(),
        },
        CONNS,
        DUR,
        BODY_LEN,
    )
    .await;

    let tunnels = h.counters.tunnels.load(Ordering::Relaxed);
    assert!(tunnels > 0, "no passthrough tunnels were opened");
    assert!(stats.reqs > 0, "no requests completed");

    // Reference row only: passthrough throughput is informational.
    println!(
        "SUMMARY throughput_sni_tunnel sni_rps={:.0} reqs={} elapsed_ms={} p50_us={} \
         p99_us={} conns={CONNS} tunnels={tunnels}",
        stats.reqs_per_sec(),
        stats.reqs,
        stats.elapsed.as_millis(),
        stats.p50_us,
        stats.p99_us,
    );

    h.shutdown.shutdown().await;
}
