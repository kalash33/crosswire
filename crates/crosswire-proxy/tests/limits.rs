//! Hostile fixtures, one per proxy-relevant `Limits` row (spec 11.6): each
//! configures its limit small and asserts the row's observable. Timing
//! bounds keep at least a 5x margin over the configured limit.

mod common;

use std::net::IpAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::relay::{body_bytes, head_bytes, read_close, wait_for, Rig, Via};
use common::upstream::spawn_raw;
use common::*;
use crosswire_h1::{ParseLimits, ParserConfig};
use crosswire_model::{Body, ExchangeState, Head, MarkerKind, TunnelReason};
use crosswire_proxy::{NoopHandler, ProxyConfig, ProxyLimits};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const OK: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok";
const ESTABLISHED: &[u8] = b"HTTP/1.1 200 Connection established\r\n\r\n";

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

fn parse_limits(
    f: impl FnOnce(crosswire_h1::ParseLimitsBuilder) -> crosswire_h1::ParseLimitsBuilder,
) -> ProxyLimits {
    let parse = ParserConfig::builder()
        .limits(f(ParseLimits::builder()).build())
        .build();
    ProxyLimits::builder().parse(parse).build()
}

/// Sends CONNECT for `authority` and returns what came back first.
async fn connect_req(addr: std::net::SocketAddr, authority: &str) -> (TcpStream, Vec<u8>) {
    let mut s = connect_raw(addr).await;
    s.write_all(format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let got = read_until(&mut s, b"\r\n\r\n").await;
    (s, got)
}

/// What a parser-limit fixture observed.
struct Refusal {
    h: ProxyHarness,
    /// Everything the client read until the close.
    reply: String,
    /// Upstream connections made (live).
    upstream_conns: Arc<AtomicUsize>,
    /// Bytes the upstream received (live).
    upstream_got: Arc<Mutex<Vec<u8>>>,
}

/// Sends `req` through a CONNECT tunnel (plain HTTP inside, so the relay
/// parses it) to a proxy with `limits`, and reads until the close.
async fn through_tunnel(limits: ProxyLimits, req: &[u8]) -> Refusal {
    let up = spawn_raw(vec![OK.to_vec()], None).await;
    let h = ProxyHarness::start_wired(
        cfg_with(limits),
        Arc::new(NoopHandler),
        wiring(up.port(), &["u.test"]),
    )
    .await;
    let (mut s, got) = connect_req(h.addr, &format!("u.test:{}", up.port())).await;
    assert_eq!(got, ESTABLISHED);
    let _ = s.write_all(req).await;
    let reply = String::from_utf8_lossy(&read_until_close(&mut s).await).into_owned();
    Refusal {
        h,
        reply,
        upstream_conns: up.connections,
        upstream_got: up.received,
    }
}

async fn wait_marker(h: &ProxyHarness, kind: MarkerKind, needle: &str) {
    wait_for(&format!("a {kind:?} marker with {needle:?}"), || {
        h.sink
            .markers
            .lock()
            .unwrap()
            .iter()
            .any(|m| m.kind() == kind && m.detail().contains(needle))
    })
    .await;
}

/// The observable of a request-head limit: a generated 400 naming the
/// error, a `LimitExceeded` marker, nothing sent upstream, nothing
/// recorded.
async fn head_refused(r: &Refusal, what: &str) {
    let (head, body) = split_response(r.reply.as_bytes());
    assert!(head.starts_with("HTTP/1.1 400 Bad Request\r\n"), "{head}");
    assert_eq!(
        String::from_utf8(body).unwrap(),
        format!("Crosswire could not parse the request: {what}")
    );
    wait_marker(
        &r.h,
        MarkerKind::LimitExceeded,
        &format!("could not be parsed: {what}"),
    )
    .await;
    // The relay connects upstream only after a head parses, and records
    // nothing for a head that does not, so neither needs a wait: both were
    // settled before the 400 was written.
    assert_eq!(r.upstream_conns.load(Ordering::SeqCst), 0);
    assert!(r.h.sink.exchanges.lock().unwrap().is_empty());
}

/// The observable of a request-body limit: a generated 400, a
/// `LimitExceeded` marker, the head (and the body bytes before the error)
/// forwarded, and the partial exchange recorded as `Error` with a cut
/// body.
async fn body_refused(r: &Refusal, req: &[u8], what: &str) -> Vec<u8> {
    let (head, body) = split_response(r.reply.as_bytes());
    assert!(head.starts_with("HTTP/1.1 400 Bad Request\r\n"), "{head}");
    assert_eq!(
        String::from_utf8(body).unwrap(),
        format!("Crosswire could not parse the request: {what}")
    );
    wait_marker(
        &r.h,
        MarkerKind::LimitExceeded,
        &format!("could not be parsed: {what}"),
    )
    .await;
    let head_len = req.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
    wait_for("the partial exchange", || {
        !r.h.sink.exchanges.lock().unwrap().is_empty()
    })
    .await;
    let ex = r.h.sink.exchanges.lock().unwrap()[0].clone();
    assert_eq!(ex.state(), ExchangeState::Error);
    assert_eq!(head_bytes(ex.request())[..], req[..head_len]);
    // What the relay says it forwarded of the body, then the upstream
    // must hold exactly the head plus that much.
    let (stored, relayed) = match ex.request().body() {
        Body::Inline { bytes, relay, .. } => {
            let relay = relay.expect("a cut body carries its relay record");
            (bytes.to_vec(), usize::try_from(relay.len).unwrap())
        }
        // Nothing of the body was relayed: no body is recorded.
        Body::None => (Vec::new(), 0),
        other => panic!("{other:?}"),
    };
    let want = head_len + relayed;
    wait_for("the forwarded bytes upstream", || {
        r.upstream_got.lock().unwrap().len() >= want
    })
    .await;
    let got = r.upstream_got.lock().unwrap().clone();
    assert_eq!(
        got[..],
        req[..want],
        "upstream got exactly what was relayed"
    );
    assert_eq!(stored[..], got[head_len..]);
    got
}

#[tokio::test]
async fn head_too_large_400_limit_marker() {
    let limits = parse_limits(|b| b.head_size(1024));
    let req = format!(
        "GET / HTTP/1.1\r\nHost: u.test\r\nX-Pad: {}\r\n\r\n",
        "a".repeat(2000)
    );
    let r = through_tunnel(limits, req.as_bytes()).await;
    head_refused(&r, "message head too large").await;
}

#[tokio::test]
async fn too_many_headers() {
    let limits = parse_limits(|b| b.max_headers(8));
    let mut req = String::from("GET / HTTP/1.1\r\nHost: u.test\r\n");
    for i in 0..9 {
        req.push_str(&format!("X-H{i}: v\r\n"));
    }
    req.push_str("\r\n");
    let r = through_tunnel(limits, req.as_bytes()).await;
    head_refused(&r, "too many headers").await;
}

#[tokio::test]
async fn line_too_long() {
    let limits = parse_limits(|b| b.max_line(64));
    let req = format!(
        "GET / HTTP/1.1\r\nHost: u.test\r\nX-Long: {}\r\n\r\n",
        "b".repeat(100)
    );
    let r = through_tunnel(limits, req.as_bytes()).await;
    head_refused(&r, "line too long").await;
}

#[tokio::test]
async fn chunk_size_line_too_long() {
    let limits = parse_limits(|b| b.max_chunk_size_line(16));
    let req = b"POST /up HTTP/1.1\r\nHost: u.test\r\nTransfer-Encoding: chunked\r\n\r\n\
                5;name=a-rather-long-extension-value\r\nhello\r\n0\r\n\r\n";
    let r = through_tunnel(limits, req).await;
    body_refused(&r, req, "line too long").await;
}

#[tokio::test]
async fn too_many_chunks() {
    let limits = parse_limits(|b| b.max_chunks(8));
    let mut req =
        b"POST /up HTTP/1.1\r\nHost: u.test\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
    for _ in 0..9 {
        req.extend_from_slice(b"1\r\nx\r\n");
    }
    req.extend_from_slice(b"0\r\n\r\n");
    let r = through_tunnel(limits, &req).await;
    let got = body_refused(&r, &req, "too many chunks").await;
    // Exactly the eight allowed chunks went upstream.
    let head_len = req.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
    assert_eq!(got.len(), head_len + 8 * b"1\r\nx\r\n".len());
}

#[tokio::test]
async fn pending_connect_cap() {
    let h = ProxyHarness::start_with_limits(ProxyLimits::builder().max_pending_connect(2).build())
        .await;
    // Two clients CONNECT and never send a ClientHello: both stay pending.
    let (c1, got1) = connect_req(h.addr, "u.test:443").await;
    let (c2, got2) = connect_req(h.addr, "u.test:443").await;
    assert_eq!(
        (got1.as_slice(), got2.as_slice()),
        (ESTABLISHED, ESTABLISHED)
    );
    // The third is closed at accept, before reading anything.
    let mut c3 = connect_raw(h.addr).await;
    let _ = c3
        .write_all(b"CONNECT u.test:443 HTTP/1.1\r\nHost: u.test:443\r\n\r\n")
        .await;
    assert_eq!(read_until_close(&mut c3).await, b"");
    assert_eq!(
        h.counters.refused_pending_connect.load(Ordering::Relaxed),
        1
    );
    assert_eq!(h.counters.accepted.load(Ordering::Relaxed), 2);
    // Once the pending ones go away, a new CONNECT is admitted again.
    drop((c1, c2));
    // The listener frees a slot when the connection task ends; retry.
    let mut admitted = false;
    for _ in 0..100 {
        let mut s = connect_raw(h.addr).await;
        let _ = s
            .write_all(b"CONNECT u.test:443 HTTP/1.1\r\nHost: u.test:443\r\n\r\n")
            .await;
        let mut buf = [0u8; 64];
        if let Ok(Ok(n)) = tokio::time::timeout(Duration::from_secs(1), s.read(&mut buf)).await {
            if buf[..n].starts_with(b"HTTP/1.1 200") {
                admitted = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        admitted,
        "a CONNECT is admitted after the pending ones closed"
    );
}

#[tokio::test]
async fn loopback_clients_are_exempt_from_per_ip_cap() {
    // Loopback clients are never capped: 70 concurrent connections with a
    // per-IP cap of 64 are all admitted. (The non-loopback side cannot be
    // reached from a loopback-only test.)
    let h =
        ProxyHarness::start_with_limits(ProxyLimits::builder().max_per_client_ip(64).build()).await;
    let mut held = Vec::new();
    for _ in 0..70 {
        let (s, got) = connect_req(h.addr, "u.test:443").await;
        assert_eq!(got, ESTABLISHED);
        held.push(s);
    }
    assert_eq!(h.counters.accepted.load(Ordering::Relaxed), 70);
    assert_eq!(h.counters.refused_per_ip.load(Ordering::Relaxed), 0);
    drop(held);
}

/// A host name of exactly `len` bytes made of valid labels.
fn long_host(len: usize) -> String {
    let mut labels = Vec::new();
    let mut left = len;
    while left > 0 {
        let take = left.min(63);
        labels.push("a".repeat(take));
        left -= take;
        if left > 0 {
            // The dot before the next label.
            left -= 1;
            if left == 0 {
                labels.last_mut().unwrap().pop();
                left = 1;
            }
        }
    }
    let host = labels.join(".");
    assert_eq!(host.len(), len);
    host
}

#[tokio::test]
async fn sni_too_long_rejects_connect() {
    let h = ProxyHarness::start_default().await;
    // 254 bytes: one over the DNS name limit. Refused with a 400 before
    // any TLS: the reply is plain HTTP and the connection closes.
    let host = long_host(254);
    let (mut s, got) = connect_req(h.addr, &format!("{host}:443")).await;
    let mut all = got;
    all.extend_from_slice(&read_until_close(&mut s).await);
    let (head, body) = split_response(&all);
    assert!(head.starts_with("HTTP/1.1 400 "), "{head}");
    assert_eq!(
        String::from_utf8(body).unwrap(),
        "Crosswire refused the request: the target is not a valid host and port."
    );
    assert_eq!(h.counters.tunnels.load(Ordering::Relaxed), 0);
    // 253 bytes is still a valid name: the tunnel is established.
    let (_s, got) = connect_req(h.addr, &format!("{}:443", long_host(253))).await;
    assert_eq!(got, ESTABLISHED);
}

/// The first TLS record a rustls client sends for `name`.
fn client_hello(name: &str) -> Vec<u8> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let cfg = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(rustls::RootCertStore::empty())
        .with_no_client_auth();
    let sn = rustls_pki_types::ServerName::try_from(name.to_string()).unwrap();
    let mut conn = rustls::ClientConnection::new(Arc::new(cfg), sn).unwrap();
    let mut out = Vec::new();
    while conn.wants_write() {
        conn.write_tls(&mut out).unwrap();
    }
    out
}

#[tokio::test]
async fn tunnel_idle_closes_passthrough() {
    // An upstream that reads and never answers.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let received = Arc::new(Mutex::new(Vec::new()));
    let rec = Arc::clone(&received);
    tokio::spawn(async move {
        while let Ok((mut tcp, _)) = listener.accept().await {
            let rec = Arc::clone(&rec);
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                while let Ok(n) = tcp.read(&mut buf).await {
                    if n == 0 {
                        break;
                    }
                    rec.lock().unwrap().extend_from_slice(&buf[..n]);
                }
            });
        }
    });
    let mut cfg = cfg_with(
        ProxyLimits::builder()
            .tunnel_idle(Some(Duration::from_millis(200)))
            .build(),
    );
    cfg.passthrough = vec!["pass.test".to_string()];
    let h =
        ProxyHarness::start_wired(cfg, Arc::new(NoopHandler), wiring(port, &["pass.test"])).await;
    let (mut s, got) = connect_req(h.addr, &format!("pass.test:{port}")).await;
    assert_eq!(got, ESTABLISHED);
    let hello = client_hello("pass.test");
    s.write_all(&hello).await.unwrap();
    let t0 = Instant::now();
    wait_for("the hello upstream", || {
        received.lock().unwrap().len() >= hello.len()
    })
    .await;
    assert_eq!(*received.lock().unwrap(), hello);
    assert_eq!(read_until_close(&mut s).await, b"");
    let elapsed = t0.elapsed();
    assert!(elapsed >= Duration::from_millis(190), "{elapsed:?}");
    assert!(elapsed < Duration::from_millis(1000), "{elapsed:?}");
    wait_for("the passthrough tunnel record", || {
        h.sink.exchanges.lock().unwrap().iter().any(|e| {
            matches!(
                e.request().head(),
                Head::Tunnel { reason: TunnelReason::SniPassthrough, bytes_in, .. }
                    if *bytes_in == hello.len() as u64
            )
        })
    })
    .await;
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

#[tokio::test]
async fn total_timeout_closes_trickle() {
    // One body byte every 100 ms, forever: never idle, never done.
    let (port, conns) = spawn_after_head(|mut s| async move {
        if s.write_all(b"HTTP/1.1 200 OK\r\n\r\n").await.is_err() {
            return;
        }
        loop {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if s.write_all(b"t").await.is_err() {
                return;
            }
        }
    })
    .await;
    let limits = ProxyLimits::builder()
        .total(Duration::from_millis(500))
        .build();
    let rig = plain_rig(port, limits, conns).await;
    let mut c = rig.open("u.test").await;
    let req = format!(
        "{}\r\nHost: u.test\r\n\r\n",
        rig.start_line("GET", "u.test", "/trickle")
    );
    // `total` runs from the request's first byte as the relay sees it
    // (`relay.rs`: `started = req_pump.first_byte()`, `deadline = started
    // + total`), which is after this instant, so the lower bound holds
    // without margin trouble; the upper bound has 5x.
    let t0 = Instant::now();
    c.write_all(req.as_bytes()).await.unwrap();
    let got = read_close(&mut c).await;
    let elapsed = t0.elapsed();
    assert!(elapsed >= Duration::from_millis(500), "{elapsed:?}");
    assert!(elapsed < Duration::from_millis(2500), "{elapsed:?}");
    assert!(got.starts_with(b"HTTP/1.1 200 OK\r\n\r\nt"), "{got:?}");
    wait_marker(
        &rig.h,
        MarkerKind::Error,
        &format!("timeout: total u.test:{port}"),
    )
    .await;
    let ex = rig.exchanges(1).await;
    assert_eq!(ex[0].state(), ExchangeState::Error);
    let body = body_bytes(ex[0].response().unwrap());
    assert!(
        !body.is_empty() && body.iter().all(|&b| b == b't'),
        "{body:?}"
    );
    assert!(body.len() <= 10, "{} bytes in 500 ms at 10/s", body.len());
}
