//! Flat-RSS proof for the proxy's body-limit rows (§13 gate #18): an
//! attacker relays far more body than `capture_cap` allows, and many
//! concurrent bodies together exceed the in-flight `body_budget`. Capture
//! memory must stay near the cap / budget, never track the volume sent.
//!
//! Not covered here: the decompression size/ratio row. The proxy never
//! decompresses a body (it records raw framed bytes); that bound lives in
//! the store's pack reader and is exercised by
//! `crosswire-store/tests/packs.rs::decompression_bomb_refused`.
//!
//! Its own test binary; every test runs its whole rig inside
//! `with_flat_rss`, is `multi_thread` (a current-thread runtime would starve
//! the sampler and pass vacuously) and streams the hostile body from one
//! reused 64 KiB buffer so the test never holds the volume itself.

mod common;
#[path = "flat_rss/mod.rs"]
mod flat_rss;

use std::net::IpAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use common::relay::{Rig, Via};
use common::*;
use crosswire_model::Body;
use crosswire_proxy::{NoopHandler, ProxyConfig, ProxyLimits};
use flat_rss::with_flat_rss;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const MIB: u64 = 1024 * 1024;

/// A plain upstream that reads one request head per connection, then runs
/// `serve`.
async fn spawn_after_head<F, Fut>(serve: F) -> (u16, Arc<AtomicUsize>)
where
    F: Fn(TcpStream) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let conns = Arc::new(AtomicUsize::new(0));
    let c = Arc::clone(&conns);
    let serve = Arc::new(serve);
    tokio::spawn(async move {
        while let Ok((mut tcp, _)) = listener.accept().await {
            let _ = tcp.set_nodelay(true);
            c.fetch_add(1, Ordering::SeqCst);
            let serve = Arc::clone(&serve);
            tokio::spawn(async move {
                let mut seen = Vec::new();
                let mut buf = [0u8; 4096];
                while !seen.windows(4).any(|w| w == b"\r\n\r\n") {
                    match tcp.read(&mut buf).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => seen.extend_from_slice(&buf[..n]),
                    }
                }
                serve(tcp).await;
            });
        }
    });
    (port, conns)
}

async fn plain_rig(port: u16, limits: ProxyLimits, connections: Arc<AtomicUsize>) -> Rig {
    let mut cfg: ProxyConfig = loopback_config();
    cfg.limits = limits;
    let h = ProxyHarness::start_wired(
        cfg,
        Arc::new(NoopHandler),
        UpstreamWiring {
            extra_roots: Vec::new(),
            allow: vec![("127.0.0.1".to_string(), port)],
            resolve: vec![("u.test".to_string(), IpAddr::from([127, 0, 0, 1]))],
            ..UpstreamWiring::default()
        },
    )
    .await;
    Rig {
        via: Via::Plain,
        h,
        port,
        received: Arc::default(),
        connections,
    }
}

/// An until-close upstream that sends `volume` bytes from a reused block.
async fn until_close_upstream(volume: u64) -> (u16, Arc<AtomicUsize>) {
    spawn_after_head(move |mut s| async move {
        if s.write_all(b"HTTP/1.0 200 OK\r\n\r\n").await.is_err() {
            return;
        }
        let block: Vec<u8> = (0..64 * 1024).map(|i| (i % 251) as u8).collect();
        let mut left = volume;
        while left > 0 {
            let n = left.min(block.len() as u64) as usize;
            if s.write_all(&block[..n]).await.is_err() {
                return;
            }
            left -= n as u64;
        }
        let _ = s.shutdown().await;
    })
    .await
}

/// Sends one GET and drains the reply with a bounded buffer.
async fn fetch_and_drain(rig: &Rig) -> u64 {
    let mut c = rig.open("u.test").await;
    let req = format!(
        "{}\r\nHost: u.test\r\n\r\n",
        rig.start_line("GET", "u.test", "/large")
    );
    c.write_all(req.as_bytes()).await.unwrap();
    let mut buf = vec![0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        match c.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => total += n as u64,
        }
    }
    total
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn capture_cap_flat_rss() {
    // 512 MiB relayed against the default 50 MiB capture cap. The kept copy
    // (50 MiB, plus a transient old buffer while it doubles) is the bound.
    const VOLUME: u64 = 512 * MIB;
    with_flat_rss(110, || async {
        let (port, conns) = until_close_upstream(VOLUME).await;
        let limits = ProxyLimits::builder().build();
        let cap = limits.capture_cap;
        assert_eq!(cap, 50 * MIB, "the row under test is the default cap");
        let rig = plain_rig(port, limits, conns).await;
        let total = tokio::time::timeout(Duration::from_secs(180), fetch_and_drain(&rig))
            .await
            .expect("the response drained in time");
        assert_eq!(total, b"HTTP/1.0 200 OK\r\n\r\n".len() as u64 + VOLUME);
        let ex = rig.exchanges(1).await;
        match ex[0].response().unwrap().body() {
            Body::Inline {
                bytes,
                truncated_at,
                capture_dropped,
                relay,
            } => {
                assert_eq!(*truncated_at, Some(cap));
                assert!(!capture_dropped);
                assert_eq!(bytes.len() as u64, cap);
                assert_eq!(relay.map(|r| r.len), Some(VOLUME));
            }
            other => panic!("{other:?}"),
        }
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inflight_budget_flat_rss() {
    // Four concurrent 128 MiB bodies (512 MiB total) against a 32 MiB
    // in-flight budget: captures that cannot grow are dropped, and the held
    // copies together stay within the budget.
    const CONNS: usize = 4;
    const VOLUME: u64 = 128 * MIB;
    with_flat_rss(64, || async {
        let (port, conns) = until_close_upstream(VOLUME).await;
        let limits = ProxyLimits::builder().body_budget(32 * MIB).build();
        let rig = Arc::new(plain_rig(port, limits, conns).await);
        let mut tasks = Vec::new();
        for _ in 0..CONNS {
            let rig = Arc::clone(&rig);
            tasks.push(tokio::spawn(async move { fetch_and_drain(&rig).await }));
        }
        for t in tasks {
            let total = tokio::time::timeout(Duration::from_secs(240), t)
                .await
                .expect("the response drained in time")
                .unwrap();
            assert_eq!(total, b"HTTP/1.0 200 OK\r\n\r\n".len() as u64 + VOLUME);
        }
        let ex = rig.exchanges(CONNS).await;
        let mut dropped = 0;
        let mut kept = 0u64;
        for e in &ex {
            match e.response().unwrap().body() {
                Body::Inline {
                    bytes,
                    capture_dropped,
                    relay,
                    ..
                } => {
                    assert_eq!(relay.map(|r| r.len), Some(VOLUME));
                    if *capture_dropped {
                        dropped += 1;
                        assert!(bytes.is_empty());
                    }
                    kept += bytes.len() as u64;
                }
                other => panic!("{other:?}"),
            }
        }
        assert!(dropped >= 1, "the budget never refused a capture");
        assert!(kept <= 32 * MIB, "held {kept} bytes over the 32 MiB budget");
    })
    .await;
}
