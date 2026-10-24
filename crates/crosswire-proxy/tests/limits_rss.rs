//! RSS while relaying a large until-close response. Its own test binary:
//! the measurement is of this process's RSS, so no other test may run in
//! parallel with it (the other `Limits` fixtures live in `limits.rs`).

mod common;

use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::relay::{Rig, Via};
use common::*;
use crosswire_model::Body;
use crosswire_proxy::{NoopHandler, ProxyConfig, ProxyLimits};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const MIB: u64 = 1024 * 1024;

fn cfg_with(limits: ProxyLimits) -> ProxyConfig {
    let mut cfg = loopback_config();
    cfg.limits = limits;
    cfg
}

fn wiring(port: u16, names: &[&str]) -> UpstreamWiring {
    UpstreamWiring {
        extra_roots: Vec::new(),
        allow: vec![("127.0.0.1".to_string(), port)],
        resolve: names
            .iter()
            .map(|n| ((*n).to_string(), IpAddr::from([127, 0, 0, 1])))
            .collect(),
        ..UpstreamWiring::default()
    }
}

/// A plain upstream that reads one request head and then runs `serve`.
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
    let h = ProxyHarness::start_wired(
        cfg_with(limits),
        Arc::new(NoopHandler),
        wiring(port, &["u.test"]),
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

/// Resident set size of this process in KiB (Linux only).
#[cfg(target_os = "linux")]
fn rss_kib() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|l| l.starts_with("VmRSS:"))?;
    line.split_whitespace().nth(1)?.parse().ok()
}

/// Elsewhere `ps` reports it; the number is printed, not asserted.
#[cfg(not(target_os = "linux"))]
fn rss_kib() -> Option<u64> {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()?;
    String::from_utf8(out.stdout).ok()?.trim().parse().ok()
}

const LARGE: u64 = 256 * MIB;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rss_flat_under_large_until_close_response() {
    // An until-close response of LARGE bytes, generated on the fly.
    let (port, conns) = spawn_after_head(|mut s| async move {
        if s.write_all(b"HTTP/1.0 200 OK\r\n\r\n").await.is_err() {
            return;
        }
        let block: Vec<u8> = (0..64 * 1024).map(|i| (i % 251) as u8).collect();
        let mut left = LARGE;
        while left > 0 {
            let n = left.min(block.len() as u64) as usize;
            if s.write_all(&block[..n]).await.is_err() {
                return;
            }
            left -= n as u64;
        }
        let _ = s.shutdown().await;
    })
    .await;
    let limits = ProxyLimits::builder().capture_cap(MIB).build();
    let rig = plain_rig(port, limits, conns).await;
    let baseline = rss_kib();
    if cfg!(target_os = "linux") {
        assert!(baseline.is_some(), "VmRSS readable from /proc/self/status");
    }
    let peak = Arc::new(AtomicU64::new(baseline.unwrap_or(0)));
    let sampler = {
        let peak = Arc::clone(&peak);
        tokio::spawn(async move {
            loop {
                if let Ok(Some(r)) = tokio::task::spawn_blocking(rss_kib).await {
                    peak.fetch_max(r, Ordering::Relaxed);
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
    };
    let t0 = Instant::now();
    let mut c = rig.open("u.test").await;
    let req = format!(
        "{}\r\nHost: u.test\r\n\r\n",
        rig.start_line("GET", "u.test", "/large")
    );
    c.write_all(req.as_bytes()).await.unwrap();
    // Drain with a bounded buffer.
    let mut buf = vec![0u8; 64 * 1024];
    let mut total = 0u64;
    tokio::time::timeout(Duration::from_secs(120), async {
        loop {
            match c.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => total += n as u64,
            }
        }
    })
    .await
    .expect("the response drained in time");
    let elapsed = t0.elapsed();
    sampler.abort();
    let head_len = b"HTTP/1.0 200 OK\r\n\r\n".len() as u64;
    assert_eq!(total, head_len + LARGE);
    let ex = rig.exchanges(1).await;
    match ex[0].response().unwrap().body() {
        Body::Inline {
            bytes,
            truncated_at,
            relay,
            ..
        } => {
            assert_eq!(*truncated_at, Some(MIB));
            assert_eq!(bytes.len() as u64, MIB);
            assert_eq!(relay.map(|r| r.len), Some(LARGE));
        }
        other => panic!("{other:?}"),
    }
    eprintln!(
        "large until-close: {} MiB relayed in {elapsed:?}",
        LARGE / MIB
    );
    if let Some(base) = baseline {
        let peak = peak.load(Ordering::Relaxed);
        let growth_mib = peak.saturating_sub(base) / 1024;
        eprintln!(
            "RSS baseline {} MiB, peak {} MiB, growth {growth_mib} MiB",
            base / 1024,
            peak / 1024
        );
        if cfg!(target_os = "linux") {
            assert!(growth_mib < 64, "RSS grew {growth_mib} MiB");
        } else {
            eprintln!("RSS assertion is Linux-only; capture bookkeeping checked");
        }
    }
}
