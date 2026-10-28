//! §13 performance gate: the proxy's steady-state **added** latency must stay
//! under 2 ms on the realistic HTTPS hot path (MITM TLS on, recording on).
//!
//! The gate measures `p99(via proxy) − p99(direct)` for an identical workload
//! — 64 keep-alive connections issuing back-to-back origin-form `GET /`
//! requests for a 16 KiB response body over client TLS — where the only
//! difference is the proxy hop. On the proxy path the client opens a `CONNECT`
//! tunnel, the proxy MITMs it (mints a leaf from its CA) and relays+records to
//! a TLS upstream; on the direct path the same client TLS goes straight to the
//! same upstream. The per-connection handshake and `CONNECT` setup happen once
//! before the measured loop, so they never enter a per-request sample.
//!
//! The run always prints a `SUMMARY added_latency ...` line. The `< 2000 µs`
//! assertion fires only under `CROSSWIRE_PERF_ASSERT=1` (set maintainer-side by
//! `just perf`), so CI smoke runs it report-only and host noise never fails an
//! unrelated change.

#[path = "load/mod.rs"]
mod load;

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{loopback_config, ProxyHarness, UpstreamWiring};
use crosswire_proxy::NoopHandler;
use load::{drive_mitm, spawn_tls_echo_upstream, TlsTarget};

/// Response body size (spec §13: 16 KiB).
const BODY_LEN: usize = 16 * 1024;
/// Keep-alive connections driven in parallel (spec §13: 64).
const CONNS: usize = 64;
/// Steady-state measurement window per path.
const DUR: Duration = Duration::from_secs(2);
/// The upstream authority the client and proxy both target.
const HOST: &str = "u.test";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn added_latency_under_2ms() {
    // One TLS echo upstream, shared by both paths: apples-to-apples, only the
    // proxy hop differs.
    let up = spawn_tls_echo_upstream(BODY_LEN).await;
    let authority = format!("{HOST}:{}", up.addr.port());

    // Baseline: client TLS straight to the upstream (trusts the upstream CA).
    let direct = drive_mitm(
        TlsTarget::Direct {
            addr: up.addr,
            ca_der: up.ca_der.clone(),
            server_name: HOST.to_string(),
        },
        CONNS,
        DUR,
        BODY_LEN,
    )
    .await;

    // A real MITM proxy with recording on: NoopHandler forwards every exchange
    // and the harness sink records it. The proxy resolves `u.test` to loopback
    // and allows the upstream port.
    let wiring = UpstreamWiring::loopback(up.ca_der.clone(), up.addr.port(), &[HOST]);
    let h = ProxyHarness::start_wired(loopback_config(), Arc::new(NoopHandler), wiring).await;

    // Via the proxy: CONNECT tunnel, then client TLS trusting the proxy CA
    // (the proxy mints the leaf), relayed and recorded to the same upstream.
    let proxy = drive_mitm(
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

    // Prove MITM + recording were genuinely active: a plain-HTTP shortcut
    // could never decrypt the tunnel, so recorded exchanges mean the proxy
    // terminated TLS, relayed, and recorded.
    let recorded = h.sink.exchanges.lock().unwrap().len();
    assert!(
        recorded > 0,
        "the proxy recorded no exchanges: MITM/recording was not active"
    );
    assert!(
        direct.reqs > 0 && proxy.reqs > 0,
        "no requests completed (direct {}, proxy {})",
        direct.reqs,
        proxy.reqs
    );

    let delta_us = proxy.p99_us as i64 - direct.p99_us as i64;
    println!(
        "SUMMARY added_latency p99_direct_us={} p99_proxy_us={} delta_us={} \
         (direct: reqs={} p50={}us {:.0}rps; proxy: reqs={} p50={}us {:.0}rps; recorded={})",
        direct.p99_us,
        proxy.p99_us,
        delta_us,
        direct.reqs,
        direct.p50_us,
        direct.reqs_per_sec(),
        proxy.reqs,
        proxy.p50_us,
        proxy.reqs_per_sec(),
        recorded,
    );

    // Report-only unless the maintainer gate asks for the hard assertion.
    if std::env::var("CROSSWIRE_PERF_ASSERT").as_deref() == Ok("1") {
        assert!(
            delta_us < 2000,
            "steady-state added p99 latency {delta_us} us exceeds the 2000 us gate \
             (direct {} us, proxy {} us)",
            direct.p99_us,
            proxy.p99_us,
        );
    }

    h.shutdown.shutdown().await;
}
