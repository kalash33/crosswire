//! Failure paths: generated 502/504/400 responses, timeouts, parse errors,
//! undelimitable and upgrade tunnels, partial exchanges, early responses
//! during uploads, and failures after a CONNECT was answered.

mod common;

use std::future::Future;
use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::relay::{body_bytes, head_bytes, read_close, read_n, rig, wait_for, Rig, Via};
use common::upstream::{spawn_raw, spawn_tls_scripted};
use common::*;
use crosswire_model::{
    Body, ChainResult, Exchange, ExchangeState, Fidelity, FramingRecord, Head, MarkerKind,
    TunnelReason,
};
use crosswire_proxy::{NoopHandler, ProxyConfig, ProxyLimits};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

fn ok(body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

/// A loopback port with nothing listening on it.
async fn dead_port() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    l.local_addr().unwrap().port()
}

/// Wiring that reaches a plain loopback upstream on `port` as `names`.
fn plain_wiring(port: u16, names: &[&str]) -> UpstreamWiring {
    UpstreamWiring {
        extra_roots: Vec::new(),
        allow: vec![("127.0.0.1".to_string(), port)],
        resolve: names
            .iter()
            .map(|n| ((*n).to_string(), IpAddr::from([127, 0, 0, 1])))
            .collect(),
    }
}

/// A proxy reaching the plain upstream on `port` as `u.test`.
async fn plain_rig(port: u16, cfg: ProxyConfig, connections: Arc<AtomicUsize>) -> Rig {
    let h = ProxyHarness::start_wired(cfg, Arc::new(NoopHandler), plain_wiring(port, &["u.test"]))
        .await;
    Rig {
        via: Via::Plain,
        h,
        port,
        received: Arc::new(Mutex::new(Vec::new())),
        connections,
    }
}

fn cfg_with(limits: ProxyLimits) -> ProxyConfig {
    let mut cfg = loopback_config();
    cfg.limits = limits;
    cfg
}

/// A plain upstream running `serve(index, stream)` for each connection.
async fn spawn_custom<F, Fut>(serve: F) -> (u16, Arc<AtomicUsize>)
where
    F: Fn(usize, TcpStream) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let connections = Arc::new(AtomicUsize::new(0));
    let conns = Arc::clone(&connections);
    let serve = Arc::new(serve);
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let _ = tcp.set_nodelay(true);
            let i = conns.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(serve(i, tcp));
        }
    });
    (port, connections)
}

/// Reads one head (through the blank line); returns it and any bytes
/// after it. Empty when the peer closes first.
async fn read_head(s: &mut TcpStream) -> (Vec<u8>, Vec<u8>) {
    let mut got = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        if let Some(i) = got.windows(4).position(|w| w == b"\r\n\r\n") {
            let rest = got.split_off(i + 4);
            return (got, rest);
        }
        match s.read(&mut buf).await {
            Ok(0) | Err(_) => return (Vec::new(), Vec::new()),
            Ok(n) => got.extend_from_slice(&buf[..n]),
        }
    }
}

/// Reads until EOF; true if EOF arrived within five seconds.
async fn closes_within_5s(s: &mut TcpStream) -> bool {
    let mut buf = vec![0u8; 64 * 1024];
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match s.read(&mut buf).await {
                Ok(0) | Err(_) => return,
                Ok(_) => {}
            }
        }
    })
    .await
    .is_ok()
}

fn get(rig: &Rig, path: &str) -> String {
    format!(
        "{}\r\nHost: u.test\r\n\r\n",
        rig.start_line("GET", "u.test", path)
    )
}

fn status_of(e: &Exchange) -> String {
    let resp = e.response().expect("a response");
    let head = head_bytes(resp);
    String::from_utf8_lossy(&head[..12]).into_owned()
}

async fn wait_marker(h: &ProxyHarness, kind: MarkerKind, needle: &str) -> String {
    let found = Arc::new(Mutex::new(String::new()));
    let f = Arc::clone(&found);
    wait_for(&format!("a {kind:?} marker with {needle:?}"), || {
        let markers = h.sink.markers.lock().unwrap();
        if let Some(m) = markers
            .iter()
            .find(|m| m.kind() == kind && m.detail().contains(needle))
        {
            *f.lock().unwrap() = m.detail().to_string();
            true
        } else {
            false
        }
    })
    .await;
    let s = found.lock().unwrap().clone();
    s
}

#[tokio::test]
async fn upstream_connect_refused_gives_502_and_error_exchange() {
    let port = dead_port().await;
    let rig = plain_rig(port, loopback_config(), Arc::default()).await;
    let mut c = rig.open("u.test").await;
    c.write_all(get(&rig, "/x").as_bytes()).await.unwrap();
    let got = read_close(&mut c).await;
    let (head, body) = split_response(&got);
    assert!(head.starts_with("HTTP/1.1 502 Bad Gateway\r\n"), "{head}");
    assert!(head.contains("\r\nX-Crosswire-Generated: 1\r\n"), "{head}");
    let body = String::from_utf8(body).unwrap();
    assert!(
        body.starts_with(&format!("Crosswire could not connect to u.test:{port}: ")),
        "{body}"
    );
    let ex = rig.exchanges(1).await;
    assert_eq!(ex[0].state(), ExchangeState::Error);
    let resp = ex[0].response().unwrap();
    assert_eq!(resp.fidelity(), Fidelity::Generated);
    assert_eq!(status_of(&ex[0]), "HTTP/1.1 502");
    assert!(head_bytes(ex[0].request()).starts_with(b"GET /x HTTP/1.1\r\n"));
}

#[tokio::test]
async fn interim_then_close_keeps_the_interim_on_the_generated_502() {
    const CONTINUE: &[u8] = b"HTTP/1.1 100 Continue\r\n\r\n";
    let (port, connections) = spawn_custom(|_, mut s| async move {
        let _ = read_head(&mut s).await;
        s.write_all(CONTINUE).await.unwrap();
        let _ = s.shutdown().await;
    })
    .await;
    let rig = plain_rig(port, loopback_config(), connections).await;
    let mut c = rig.open("u.test").await;
    c.write_all(get(&rig, "/i").as_bytes()).await.unwrap();
    let got = read_close(&mut c).await;
    assert!(got.starts_with(CONTINUE), "{got:?}");
    assert!(got[CONTINUE.len()..].starts_with(b"HTTP/1.1 502 Bad Gateway\r\n"));
    let ex = rig.exchanges(1).await;
    assert_eq!(ex[0].state(), ExchangeState::Error);
    let resp = ex[0].response().unwrap();
    assert_eq!(resp.fidelity(), Fidelity::Generated);
    // The client saw the 100 before the 502: the record keeps it.
    assert_eq!(resp.interim().len(), 1);
    assert_eq!(&resp.interim()[0][..], CONTINUE);
}

#[tokio::test]
async fn connect_to_own_listener_is_refused_with_hint() {
    let h = ProxyHarness::start_default().await;
    let port = h.addr.port();
    let rig = Rig {
        via: Via::Tls,
        h,
        port,
        received: Arc::default(),
        connections: Arc::default(),
    };
    let mut c = rig.open("127.0.0.1").await;
    let req = format!("GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n");
    c.write_all(req.as_bytes()).await.unwrap();
    let got = read_close(&mut c).await;
    let (head, body) = split_response(&got);
    assert!(head.starts_with("HTTP/1.1 502 Bad Gateway\r\n"), "{head}");
    let body = String::from_utf8(body).unwrap();
    assert!(
        body.starts_with(&format!("Crosswire refused 127.0.0.1:{port} (")),
        "{body}"
    );
    assert!(
        body.contains(&format!("--allow 127.0.0.1:{port}")),
        "{body}"
    );
    let ex = rig.exchanges(1).await;
    assert_eq!(ex[0].state(), ExchangeState::Error);
    // Only the client's own connection: the proxy never dialled itself.
    assert_eq!(rig.h.counters.accepted.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn upstream_bad_chain_is_502_without_override_and_relayed_with_it() {
    let up = spawn_tls_scripted(vec![ok("fine")], None).await;
    let port = up.port();
    // The upstream's CA is not trusted by either proxy.
    let wiring = plain_wiring(port, &["u.test"]);

    let strict =
        ProxyHarness::start_wired(loopback_config(), Arc::new(NoopHandler), wiring.clone()).await;
    let rig1 = Rig {
        via: Via::Tls,
        h: strict,
        port,
        received: Arc::default(),
        connections: Arc::default(),
    };
    let mut c = rig1.open("u.test").await;
    c.write_all(get(&rig1, "/a").as_bytes()).await.unwrap();
    let got = read_close(&mut c).await;
    let (head, body) = split_response(&got);
    assert!(head.starts_with("HTTP/1.1 502 Bad Gateway\r\n"), "{head}");
    let body = String::from_utf8(body).unwrap();
    assert!(
        body.starts_with(&format!(
            "Crosswire refused the upstream certificate for u.test:{port}: "
        )),
        "{body}"
    );
    assert!(
        body.contains("Pass --accept-invalid-cert u.test to override"),
        "{body}"
    );
    let ex = rig1.exchanges(1).await;
    assert_eq!(ex[0].state(), ExchangeState::Error);

    let mut cfg = loopback_config();
    cfg.accept_invalid_cert = vec!["u.test".to_string()];
    let lax = ProxyHarness::start_wired(cfg, Arc::new(NoopHandler), wiring).await;
    let rig2 = Rig {
        via: Via::Tls,
        h: lax,
        port,
        received: Arc::default(),
        connections: Arc::default(),
    };
    let mut c = rig2.open("u.test").await;
    c.write_all(get(&rig2, "/b").as_bytes()).await.unwrap();
    assert_eq!(read_n(&mut c, ok("fine").len()).await, ok("fine"));
    let ex = rig2.exchanges(1).await;
    assert_eq!(ex[0].state(), ExchangeState::Received);
    let conn = ex[0].request().connection();
    match conn.upstream_chain_result() {
        ChainResult::Overridden { reason } => assert!(!reason.is_empty()),
        other => panic!("expected an override, got {other:?}"),
    }
    assert!(conn.cert_override());
}

#[tokio::test]
async fn response_head_timeout_gives_504() {
    let (port, conns) = spawn_custom(|_, mut s| async move {
        // Read forever, never answer.
        let mut buf = [0u8; 4096];
        while matches!(s.read(&mut buf).await, Ok(n) if n > 0) {}
    })
    .await;
    let limits = ProxyLimits::builder()
        .idle(Duration::from_millis(300))
        .build();
    let rig = plain_rig(port, cfg_with(limits), conns).await;
    let mut c = rig.open("u.test").await;
    let t0 = Instant::now();
    c.write_all(get(&rig, "/slow").as_bytes()).await.unwrap();
    let got = read_close(&mut c).await;
    let elapsed = t0.elapsed();
    assert!(elapsed >= Duration::from_millis(300), "{elapsed:?}");
    assert!(elapsed < Duration::from_millis(1500), "{elapsed:?}");
    let (head, body) = split_response(&got);
    assert!(
        head.starts_with("HTTP/1.1 504 Gateway Timeout\r\n"),
        "{head}"
    );
    assert_eq!(
        String::from_utf8(body).unwrap(),
        format!("Crosswire timed out waiting for u.test:{port} (response head)")
    );
    let ex = rig.exchanges(1).await;
    assert_eq!(ex[0].state(), ExchangeState::Error);
    assert_eq!(status_of(&ex[0]), "HTTP/1.1 504");
}

#[tokio::test]
async fn request_head_over_limit_gives_400_and_limit_marker() {
    let up = spawn_tls_scripted(vec![ok("never")], None).await;
    let rig = Rig {
        via: Via::Tls,
        h: ProxyHarness::start_wired(
            loopback_config(),
            Arc::new(NoopHandler),
            UpstreamWiring::loopback(up.ca_der(), up.port(), &["u.test"]),
        )
        .await,
        port: up.port(),
        received: Arc::clone(&up.received),
        connections: Arc::clone(&up.connections),
    };
    let mut c = rig.open("u.test").await;
    let mut req = String::from("GET / HTTP/1.1\r\nHost: u.test\r\n");
    for i in 0..72 {
        req.push_str(&format!("X-Pad-{i}: {}\r\n", "a".repeat(1000)));
    }
    req.push_str("\r\n");
    assert!(req.len() > 70 * 1024);
    let _ = c.write_all(req.as_bytes()).await;
    let got = read_close(&mut c).await;
    let (head, body) = split_response(&got);
    assert!(head.starts_with("HTTP/1.1 400 Bad Request\r\n"), "{head}");
    assert_eq!(
        String::from_utf8(body).unwrap(),
        "Crosswire could not parse the request: message head too large"
    );
    wait_marker(&rig.h, MarkerKind::LimitExceeded, "message head too large").await;
    assert_eq!(rig.connections(), 0);
}

/// `tunnel_out` is how many response bytes cross after the framing broke:
/// read-ahead flushed into the tunnel counts as tunnel traffic.
async fn undelimitable_case(via: Via, resp: Vec<u8>, tunnel_out: usize) {
    let rig = rig(via, vec![resp.clone()], None, &["u.test"]).await;
    let mut c = rig.open("u.test").await;
    let req = get(&rig, "/u");
    c.write_all(req.as_bytes()).await.unwrap();
    assert_eq!(read_n(&mut c, resp.len()).await, resp);
    // The connection is a raw tunnel now: client bytes reach the upstream.
    c.write_all(b"after").await.unwrap();
    // The upstream saw the request (origin-form) and then the raw bytes.
    wait_for("the raw bytes upstream", || {
        rig.received.lock().unwrap().ends_with(b"after")
    })
    .await;
    drop(c);
    let ex = rig.exchanges(2).await;
    let http = ex
        .iter()
        .find(|e| !matches!(e.request().head(), Head::Tunnel { .. }))
        .unwrap();
    assert_eq!(http.state(), ExchangeState::Error);
    assert!(head_bytes(http.request()).starts_with(b"GET /u HTTP/1.1\r\n"));
    let tunnel = ex
        .iter()
        .find(|e| matches!(e.request().head(), Head::Tunnel { .. }))
        .expect("a tunnel exchange");
    let Head::Tunnel {
        reason,
        bytes_in,
        bytes_out,
        ..
    } = tunnel.request().head()
    else {
        unreachable!()
    };
    assert_eq!(*reason, TunnelReason::Undelimitable);
    assert_eq!(*bytes_in, 5);
    assert_eq!(*bytes_out, tunnel_out as u64);
    // The response head is recorded once, as the head; the body holds
    // only bytes after it.
    let r = http.response().expect("the response so far");
    let head = head_bytes(r);
    assert!(resp.starts_with(&head), "{head:?}");
    let body = body_bytes(r);
    assert!(resp[head.len()..].starts_with(&body), "{body:?}");
    wait_marker(
        &rig.h,
        MarkerKind::Error,
        &format!("undelimitable response from u.test:{}: ", rig.port),
    )
    .await;
}

#[tokio::test]
async fn undelimitable_response_falls_to_tunnel() {
    let garbage = b"BOGUS garbage status line\r\nX: y\r\n\r\nraw bytes follow".to_vec();
    let n = garbage.len();
    undelimitable_case(Via::Plain, garbage.clone(), n).await;
    undelimitable_case(Via::Tls, garbage, n).await;
}

#[tokio::test]
async fn invalid_chunk_in_response_body_falls_to_tunnel() {
    let resp =
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\nZZ\r\nrest".to_vec();
    let n = b"ZZ\r\nrest".len();
    undelimitable_case(Via::Plain, resp, n).await;
}

const SWITCH: &[u8] =
    b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n";

async fn switching_case(via: Via) {
    let rig = rig(
        via,
        vec![SWITCH.to_vec(), b"PONG".to_vec()],
        None,
        &["u.test"],
    )
    .await;
    let tunnels_before = rig.h.counters.tunnels.load(Ordering::SeqCst);
    let mut c = rig.open("u.test").await;
    let req = format!(
        "{}\r\nHost: u.test\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n",
        rig.start_line("GET", "u.test", "/ws")
    );
    c.write_all(req.as_bytes()).await.unwrap();
    assert_eq!(read_n(&mut c, SWITCH.len()).await, SWITCH);
    // The upstream answers a request-shaped frame with PONG.
    let ping = b"PING /f HTTP/1.1\r\n\r\n";
    c.write_all(ping).await.unwrap();
    assert_eq!(read_n(&mut c, 4).await, b"PONG");
    drop(c);
    let ex = rig.exchanges(2).await;
    let upgrade = ex
        .iter()
        .find(|e| !matches!(e.request().head(), Head::Tunnel { .. }))
        .unwrap();
    assert_eq!(upgrade.state(), ExchangeState::Received);
    let resp = upgrade.response().unwrap();
    assert_eq!(head_bytes(resp), SWITCH);
    assert_eq!(*resp.framing(), FramingRecord::Upgrade);
    let tunnel = ex
        .iter()
        .find(|e| matches!(e.request().head(), Head::Tunnel { .. }))
        .unwrap();
    let Head::Tunnel {
        reason,
        bytes_in,
        bytes_out,
        ..
    } = tunnel.request().head()
    else {
        unreachable!()
    };
    assert_eq!(*reason, TunnelReason::Ws);
    assert_eq!(*bytes_in, ping.len() as u64);
    assert_eq!(*bytes_out, 4);
    assert_eq!(
        rig.h.counters.tunnels.load(Ordering::SeqCst),
        tunnels_before + 1
    );
}

#[tokio::test]
async fn switching_protocols_records_upgrade_then_ws_tunnel() {
    switching_case(Via::Plain).await;
    switching_case(Via::Tls).await;
}

#[tokio::test]
async fn upstream_close_mid_body_records_error_and_closes_client() {
    let resp =
        b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\npartial".to_vec();
    let rig = rig(Via::Plain, vec![resp.clone()], None, &["u.test"]).await;
    let mut c = rig.open("u.test").await;
    c.write_all(get(&rig, "/short").as_bytes()).await.unwrap();
    assert_eq!(read_close(&mut c).await, resp);
    let ex = rig.exchanges(1).await;
    assert_eq!(ex[0].state(), ExchangeState::Error);
    assert_eq!(body_bytes(ex[0].response().unwrap()), &b"partial"[..]);
    let detail = wait_marker(&rig.h, MarkerKind::Error, "unexpected end of input").await;
    assert!(detail.contains(&format!("u.test:{}", rig.port)), "{detail}");
}

#[tokio::test]
async fn client_disconnect_mid_request_body_records_partial_request() {
    let received = Arc::new(Mutex::new(Vec::new()));
    let closed = Arc::new(AtomicBool::new(false));
    let (rec, cl) = (Arc::clone(&received), Arc::clone(&closed));
    let (port, conns) = spawn_custom(move |_, mut s| {
        let (rec, cl) = (Arc::clone(&rec), Arc::clone(&cl));
        async move {
            let mut buf = [0u8; 4096];
            while let Ok(n) = s.read(&mut buf).await {
                if n == 0 {
                    break;
                }
                rec.lock().unwrap().extend_from_slice(&buf[..n]);
            }
            cl.store(true, Ordering::SeqCst);
        }
    })
    .await;
    let rig = plain_rig(port, loopback_config(), conns).await;
    let mut c = rig.open("u.test").await;
    let head = format!(
        "{}\r\nHost: u.test\r\nContent-Length: 1000\r\n\r\n",
        rig.start_line("POST", "u.test", "/up")
    );
    c.write_all(head.as_bytes()).await.unwrap();
    c.write_all(&[b'x'; 100]).await.unwrap();
    wait_for("100 body bytes upstream", || {
        received.lock().unwrap().ends_with(&[b'x'; 100])
    })
    .await;
    drop(c);
    let ex = rig.exchanges(1).await;
    assert_eq!(ex[0].state(), ExchangeState::Error);
    assert!(ex[0].response().is_none());
    match ex[0].request().body() {
        Body::Inline { bytes, relay, .. } => {
            assert_eq!(bytes.len(), 100);
            assert_eq!(
                relay.map(|r| r.len),
                Some(100),
                "a cut body carries its relay record"
            );
        }
        other => panic!("expected an inline body, got {other:?}"),
    }
    wait_for("the upstream side closed", || closed.load(Ordering::SeqCst)).await;
}

#[tokio::test]
async fn tunnel_idle_closes() {
    let limits = ProxyLimits::builder()
        .tunnel_idle(Some(Duration::from_millis(200)))
        .build();
    let up = spawn_raw(vec![SWITCH.to_vec()], None).await;
    let h = ProxyHarness::start_wired(
        cfg_with(limits),
        Arc::new(NoopHandler),
        plain_wiring(up.port(), &["u.test"]),
    )
    .await;
    let rig = Rig {
        via: Via::Plain,
        h,
        port: up.port(),
        received: up.received,
        connections: up.connections,
    };
    let mut c = rig.open("u.test").await;
    c.write_all(get(&rig, "/ws").as_bytes()).await.unwrap();
    assert_eq!(read_n(&mut c, SWITCH.len()).await, SWITCH);
    let t0 = Instant::now();
    assert_eq!(read_close(&mut c).await, b"");
    let elapsed = t0.elapsed();
    assert!(elapsed >= Duration::from_millis(150), "{elapsed:?}");
    assert!(elapsed < Duration::from_millis(1000), "{elapsed:?}");
    rig.exchanges(2).await;
}

#[tokio::test]
async fn expect_100_continue_client_waits_for_interim() {
    let (port, conns) = spawn_custom(|_, mut s| async move {
        let (head, mut body) = read_head(&mut s).await;
        assert!(String::from_utf8_lossy(&head).contains("Expect: 100-continue"));
        s.write_all(b"HTTP/1.1 100 Continue\r\n\r\n").await.unwrap();
        let mut buf = [0u8; 64];
        while body.len() < 4 {
            let n = s.read(&mut buf).await.unwrap();
            assert!(n > 0);
            body.extend_from_slice(&buf[..n]);
        }
        assert_eq!(body, b"data");
        s.write_all(&ok("done")).await.unwrap();
        let _ = closes_within_5s(&mut s).await;
    })
    .await;
    let rig = plain_rig(port, loopback_config(), conns).await;
    let mut c = connect_raw(rig.h.addr).await;
    let head = format!(
        "{}\r\nHost: u.test\r\nExpect: 100-continue\r\nContent-Length: 4\r\n\r\n",
        rig.start_line("POST", "u.test", "/e")
    );
    c.write_all(head.as_bytes()).await.unwrap();
    let interim = read_until(&mut c, b"\r\n\r\n").await;
    assert_eq!(interim, b"HTTP/1.1 100 Continue\r\n\r\n");
    c.write_all(b"data").await.unwrap();
    let want = ok("done");
    let mut got = vec![0u8; want.len()];
    tokio::time::timeout(Duration::from_secs(5), c.read_exact(&mut got))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got, want);
    let ex = rig.exchanges(1).await;
    assert_eq!(ex[0].state(), ExchangeState::Received);
    assert_eq!(ex[0].response().unwrap().interim().len(), 1);
    assert_eq!(body_bytes(ex[0].request()), &b"data"[..]);
}

#[tokio::test]
async fn early_413_during_upload_is_forwarded_and_closes() {
    let up_closed = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&up_closed);
    let (port, conns) = spawn_custom(move |_, mut s| {
        let flag = Arc::clone(&flag);
        async move {
            let (_head, mut body) = read_head(&mut s).await;
            let mut buf = [0u8; 1024];
            while body.len() < 1024 {
                let n = s.read(&mut buf).await.unwrap();
                assert!(n > 0);
                body.extend_from_slice(&buf[..n]);
            }
            s.write_all(b"HTTP/1.1 413 Payload Too Large\r\nContent-Length: 0\r\n\r\n")
                .await
                .unwrap();
            // Stop reading for a while: the proxy must not depend on it.
            tokio::time::sleep(Duration::from_millis(300)).await;
            if closes_within_5s(&mut s).await {
                flag.store(true, Ordering::SeqCst);
            }
        }
    })
    .await;
    let rig = plain_rig(port, loopback_config(), conns).await;
    let c = connect_raw(rig.h.addr).await;
    let (mut rd, mut wr) = c.into_split();
    let head = format!(
        "{}\r\nHost: u.test\r\nContent-Length: {}\r\n\r\n",
        rig.start_line("POST", "u.test", "/big"),
        1024 * 1024
    );
    let writer = tokio::spawn(async move {
        let _ = wr.write_all(head.as_bytes()).await;
        let chunk = vec![b'z'; 16 * 1024];
        for _ in 0..64 {
            if wr.write_all(&chunk).await.is_err() {
                break;
            }
        }
    });
    let mut got = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut buf = [0u8; 4096];
        loop {
            match rd.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => got.extend_from_slice(&buf[..n]),
            }
        }
    })
    .await
    .expect("client side closed");
    assert_eq!(
        got,
        b"HTTP/1.1 413 Payload Too Large\r\nContent-Length: 0\r\n\r\n"
    );
    let ex = rig.exchanges(1).await;
    assert_eq!(status_of(&ex[0]), "HTTP/1.1 413");
    match ex[0].request().body() {
        Body::Inline { bytes, relay, .. } => {
            assert!(bytes.len() < 1024 * 1024);
            assert!(relay.is_some(), "the cut body carries its relay record");
        }
        other => panic!("expected an inline body, got {other:?}"),
    }
    wait_for("the upstream side closed", || {
        up_closed.load(Ordering::SeqCst)
    })
    .await;
    writer.abort();
}

/// The reused upstream drops request /2 unanswered (a FIN, or a reset
/// with `reset`); `extra` is added to the head of /2.
async fn retry_case(reset: bool, extra: &'static str) {
    let (port, conns) = spawn_custom(move |i, mut s| async move {
        if i == 0 {
            let (h, _) = read_head(&mut s).await;
            assert!(h.starts_with(b"GET /1 "));
            s.write_all(&ok("one")).await.unwrap();
            // The next request arrives just as this side gives up.
            let (h, _) = read_head(&mut s).await;
            assert!(h.starts_with(b"GET /2 "));
            if reset {
                #[allow(deprecated, reason = "zero linger is how a test sends a reset")]
                s.set_linger(Some(Duration::ZERO)).unwrap();
            }
            drop(s);
        } else {
            let (h, _) = read_head(&mut s).await;
            assert!(h.starts_with(b"GET /2 "));
            s.write_all(&ok("two")).await.unwrap();
            let _ = closes_within_5s(&mut s).await;
        }
    })
    .await;
    let rig = plain_rig(port, loopback_config(), conns).await;
    let mut c = rig.open("u.test").await;
    c.write_all(get(&rig, "/1").as_bytes()).await.unwrap();
    assert_eq!(read_n(&mut c, ok("one").len()).await, ok("one"));
    let second = format!(
        "{}\r\nHost: u.test\r\n{extra}\r\n",
        rig.start_line("GET", "u.test", "/2")
    );
    c.write_all(second.as_bytes()).await.unwrap();
    assert_eq!(read_n(&mut c, ok("two").len()).await, ok("two"));
    let ex = rig.exchanges(2).await;
    assert!(ex.iter().all(|e| e.state() == ExchangeState::Received));
    assert_eq!(rig.connections(), 2);
}

#[tokio::test]
async fn idle_closed_upstream_is_retried_once_for_get() {
    retry_case(false, "").await;
}

#[tokio::test]
async fn reset_reused_upstream_is_retried_once_for_get() {
    retry_case(true, "").await;
}

#[tokio::test]
async fn content_length_zero_counts_as_bodiless_for_retry() {
    retry_case(false, "Content-Length: 0\r\n").await;
}

/// A request body of big-endian u32 counters: any duplicated or lost run
/// breaks the sequence.
fn counters(len: usize) -> Vec<u8> {
    (0..u32::try_from(len / 4).unwrap())
        .flat_map(u32::to_be_bytes)
        .collect()
}

#[tokio::test]
async fn early_garbage_head_during_upload_does_not_duplicate_body_bytes() {
    const LEN: usize = 1024 * 1024;
    let body_seen = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&body_seen);
    let (port, conns) = spawn_custom(move |_, mut s| {
        let seen = Arc::clone(&seen);
        async move {
            let (_head, mut body) = read_head(&mut s).await;
            let mut buf = vec![0u8; 64 * 1024];
            while body.len() < 100 * 1024 {
                let n = s.read(&mut buf).await.unwrap();
                assert!(n > 0);
                body.extend_from_slice(&buf[..n]);
            }
            s.write_all(b"BOGUS garbage\r\n\r\n").await.unwrap();
            // Stop reading so the proxy's body write stalls mid-way, then
            // take everything else the tunnel brings.
            tokio::time::sleep(Duration::from_millis(200)).await;
            loop {
                match s.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => body.extend_from_slice(&buf[..n]),
                }
            }
            *seen.lock().unwrap() = body;
        }
    })
    .await;
    let rig = plain_rig(port, loopback_config(), conns).await;
    let c = connect_raw(rig.h.addr).await;
    let (mut rd, mut wr) = c.into_split();
    let head = format!(
        "{}\r\nHost: u.test\r\nContent-Length: {LEN}\r\n\r\n",
        rig.start_line("POST", "u.test", "/up")
    );
    let writer = tokio::spawn(async move {
        wr.write_all(head.as_bytes()).await.unwrap();
        wr.write_all(&counters(LEN)).await.unwrap();
        wr.shutdown().await.unwrap();
        wr
    });
    let want_head = b"BOGUS garbage\r\n\r\n";
    let mut got = vec![0u8; want_head.len()];
    tokio::time::timeout(Duration::from_secs(5), rd.read_exact(&mut got))
        .await
        .expect("the garbage head in time")
        .unwrap();
    assert_eq!(got, want_head);
    let _wr = tokio::time::timeout(Duration::from_secs(5), writer)
        .await
        .expect("upload finished")
        .unwrap();
    wait_for("the upstream saw the whole upload", || {
        !body_seen.lock().unwrap().is_empty()
    })
    .await;
    let body = body_seen.lock().unwrap().clone();
    let want = counters(LEN);
    if body != want {
        let first_bad = body.chunks(4).zip(want.chunks(4)).position(|(a, b)| a != b);
        panic!(
            "upstream got {} bytes (want {LEN}); first mismatching counter at {first_bad:?}",
            body.len()
        );
    }
}

#[tokio::test]
async fn connect_then_plain_http_is_relayed() {
    let up = spawn_raw(vec![ok("plain")], None).await;
    let h = ProxyHarness::start_wired(
        loopback_config(),
        Arc::new(NoopHandler),
        plain_wiring(up.port(), &["u.test"]),
    )
    .await;
    let mut s = connect_raw(h.addr).await;
    let authority = format!("u.test:{}", up.port());
    s.write_all(format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n\r\n").as_bytes())
        .await
        .unwrap();
    assert_eq!(
        read_until(&mut s, b"\r\n\r\n").await,
        b"HTTP/1.1 200 Connection established\r\n\r\n"
    );
    s.write_all(b"GET /p HTTP/1.1\r\nHost: u.test\r\n\r\n")
        .await
        .unwrap();
    let want = ok("plain");
    let mut got = vec![0u8; want.len()];
    tokio::time::timeout(Duration::from_secs(5), s.read_exact(&mut got))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got, want);
    wait_for("one exchange", || {
        h.sink.exchanges.lock().unwrap().len() == 1
    })
    .await;
    let ex = h.sink.exchanges.lock().unwrap().clone();
    assert!(head_bytes(ex[0].request()).starts_with(b"GET /p HTTP/1.1\r\n"));
}

#[tokio::test]
async fn passthrough_upstream_failure_records_marker() {
    let port = dead_port().await;
    let mut cfg = loopback_config();
    cfg.passthrough = vec!["pass.test".to_string()];
    let h = ProxyHarness::start_wired(
        cfg,
        Arc::new(NoopHandler),
        plain_wiring(port, &["pass.test"]),
    )
    .await;
    let rig = Rig {
        via: Via::Tls,
        h,
        port,
        received: Arc::default(),
        connections: Arc::default(),
    };
    // The handshake cannot complete; open() would panic, so drive it here.
    let task = tokio::spawn({
        let addr = rig.h.addr;
        async move {
            let mut s = connect_raw(addr).await;
            let authority = format!("pass.test:{port}");
            s.write_all(
                format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n\r\n").as_bytes(),
            )
            .await
            .unwrap();
            let _ = read_until(&mut s, b"\r\n\r\n").await;
            // A ClientHello-shaped record carrying SNI pass.test.
            let hello = client_hello("pass.test");
            s.write_all(&hello).await.unwrap();
            closes_within_5s(&mut s).await
        }
    });
    let detail = wait_marker(
        &rig.h,
        MarkerKind::Error,
        &format!("passthrough upstream failed for pass.test:{port}: "),
    )
    .await;
    assert!(!detail.is_empty());
    assert!(task.await.unwrap(), "the client side was closed");
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
