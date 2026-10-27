//! Flat-RSS proof for the `intercept_hold` row (§13 gate #18): a held request
//! whose body is far over the hold cap is NOT buffered (the held copy is
//! bounded by the cap, never by the volume the attacker sends), and anonymous
//! RSS stays flat while the body streams through.
//!
//! `multi_thread` (a current-thread runtime would starve the sampler and pass
//! vacuously); the hostile body streams from one reused 64 KiB block so the
//! test never holds the volume itself, and the upstream discards what it reads.

mod common;
#[path = "flat_rss/mod.rs"]
mod flat_rss;

use std::net::IpAddr;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use common::relay::wait_for;
use common::{connect_raw, loopback_config, ProxyHarness, TestSink, UpstreamWiring};
use crosswire_model::ExchangeSink;
use crosswire_proxy::{Budget, Handler, Intercept, InterceptRules, ProxyLimits};
use crosswire_scope::Scope;
use flat_rss::with_flat_rss;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const MIB: u64 = 1024 * 1024;

/// An upstream that reads one request (head, then `body_len` body bytes),
/// discarding the body, and answers 200.
async fn discarding_upstream(body_len: u64) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut tcp, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut head = Vec::new();
                let mut buf = vec![0u8; 64 * 1024];
                let mut body_seen = 0u64;
                loop {
                    let n = match tcp.read(&mut buf).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => n,
                    };
                    if head.windows(4).any(|w| w == b"\r\n\r\n") {
                        body_seen += n as u64;
                    } else {
                        head.extend_from_slice(&buf[..n]);
                        if let Some(end) = head.windows(4).position(|w| w == b"\r\n\r\n") {
                            body_seen += (head.len() - (end + 4)) as u64;
                        }
                    }
                    if head.windows(4).any(|w| w == b"\r\n\r\n") && body_seen >= body_len {
                        break;
                    }
                }
                let _ = tcp
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                    )
                    .await;
                let _ = tcp.shutdown().await;
            });
        }
    });
    port
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn intercept_hold_flat_rss() {
    // 256 MiB body against the default 4 MiB hold cap (64x over). The capture
    // cap is lowered to 8 MiB (a config knob, not a limit change) so the
    // request capture does not mask the held copy: growth must stay near the
    // caps, not near 256 MiB.
    const VOLUME: u64 = 256 * MIB;
    with_flat_rss(48, || async {
        let port = discarding_upstream(VOLUME).await;
        let limits = ProxyLimits::builder().capture_cap(8 * MIB).build();
        assert_eq!(limits.intercept_hold, 4 * MIB, "the row under test is the default hold cap");
        let scope = Arc::new(RwLock::new(Scope::default().compile().unwrap()));
        let intercept = Arc::new(Intercept::new(
            &limits,
            Arc::clone(&scope),
            InterceptRules::default(),
            Arc::new(Budget::new(limits.body_budget)),
            Arc::new(TestSink::default()) as Arc<dyn ExchangeSink>,
        ));
        intercept.set_enabled(true);
        let mut cfg = loopback_config();
        cfg.limits = limits;
        let wiring = UpstreamWiring {
            extra_roots: Vec::new(),
            allow: vec![("127.0.0.1".to_string(), port)],
            resolve: vec![("u.test".to_string(), IpAddr::from([127, 0, 0, 1]))],
            deny_unknown: false,
        };
        let handler: Arc<dyn Handler> = Arc::clone(&intercept) as Arc<dyn Handler>;
        let h = ProxyHarness::start_scoped(cfg, handler, wiring, Arc::clone(&scope)).await;

        let head = format!(
            "POST http://u.test:{port}/up HTTP/1.1\r\nHost: u.test\r\nContent-Length: {VOLUME}\r\n\r\n"
        );
        let mut c = connect_raw(h.addr).await;
        let (mut rd, mut wr) = c.split();
        let writer = async {
            wr.write_all(head.as_bytes()).await.unwrap();
            let block: Vec<u8> = (0..64 * 1024).map(|i| (i % 251) as u8).collect();
            let mut left = VOLUME;
            while left > 0 {
                let n = left.min(block.len() as u64) as usize;
                if wr.write_all(&block[..n]).await.is_err() {
                    return;
                }
                left -= n as u64;
            }
            let _ = wr.flush().await;
        };
        let drive = async {
            let i = Arc::clone(&intercept);
            wait_for("the request to be held", move || i.held().len() == 1).await;
            let held = intercept.held();
            assert!(!held[0].editable, "a body over the hold cap is not editable");
            assert_eq!(held[0].size, 0, "no body bytes are held for an over-cap body");
            let bytes = intercept.held_bytes(held[0].id).expect("held bytes");
            assert!(
                bytes.body.as_ref().map_or(0, |b| b.len() as u64) <= 4 * MIB,
                "held body bytes exceed the 4 MiB cap"
            );
            assert!(bytes.body.is_none(), "an over-cap body is not buffered");
            intercept.forward(held[0].id, None).unwrap();
        };
        let read = async {
            let mut got = Vec::new();
            let mut buf = [0u8; 8192];
            tokio::time::timeout(Duration::from_secs(180), async {
                loop {
                    match rd.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => got.extend_from_slice(&buf[..n]),
                    }
                }
            })
            .await
            .expect("response in time");
            assert!(String::from_utf8_lossy(&got).contains("200 OK"));
        };
        tokio::join!(writer, drive, read);
    })
    .await;
}
