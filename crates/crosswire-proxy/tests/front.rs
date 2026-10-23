//! The listener front end: CONNECT, direct requests, the info page, proxy
//! auth and connection caps.

mod common;

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use common::*;
use crosswire_proxy::{NoopHandler, Proxy, ProxyAuth, ProxyConfig, ProxyError, ProxyLimits};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn connect_gets_200_connection_established() {
    let h = ProxyHarness::start_default().await;
    let mut s = connect_raw(h.addr).await;
    s.write_all(b"CONNECT example.test:443 HTTP/1.1\r\nHost: example.test:443\r\n\r\n")
        .await
        .unwrap();
    let got = read_until(&mut s, b"\r\n\r\n").await;
    assert_eq!(got, b"HTTP/1.1 200 Connection established\r\n\r\n");
}

#[tokio::test]
async fn origin_form_to_unknown_host_is_refused_400() {
    let h = ProxyHarness::start_default().await;
    let resp =
        send_and_read_until_close(h.addr, b"GET / HTTP/1.1\r\nHost: example.test\r\n\r\n").await;
    let (head, body) = split_response(&resp);
    assert!(head.starts_with("HTTP/1.1 400 "), "{head}");
    assert!(String::from_utf8_lossy(&body).contains("Crosswire is a proxy"));
}

#[tokio::test]
async fn info_page_served_for_host_crosswire() {
    let h = ProxyHarness::start_default().await;
    let resp =
        send_and_read_until_close(h.addr, b"GET / HTTP/1.1\r\nHost: crosswire\r\n\r\n").await;
    let (head, body) = split_response(&resp);
    assert!(head.starts_with("HTTP/1.1 200 "), "{head}");
    let lower = head.to_ascii_lowercase();
    assert!(lower.contains("content-type: text/html"), "{head}");
    assert!(lower.contains("x-content-type-options: nosniff"), "{head}");
    assert!(
        head.contains("Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'"),
        "{head}"
    );
    assert!(head.contains("Referrer-Policy: no-referrer"), "{head}");
    let body = String::from_utf8(body).unwrap();
    assert!(body.contains("/ca.der"));
    assert!(body.contains("/ca.pem"));
    assert!(body.contains("/ca.spki"));
    assert!(!body.to_ascii_lowercase().contains("<script"));

    // The listener's own host:port reaches the page too.
    let req = format!("GET / HTTP/1.1\r\nHost: {}\r\n\r\n", h.addr);
    let resp = send_and_read_until_close(h.addr, req.as_bytes()).await;
    assert!(resp.starts_with(b"HTTP/1.1 200 "));

    // Unknown paths on the local host are 404.
    let resp =
        send_and_read_until_close(h.addr, b"GET /nope HTTP/1.1\r\nHost: crosswire\r\n\r\n").await;
    assert!(resp.starts_with(b"HTTP/1.1 404 "));
}

#[tokio::test]
async fn ca_der_matches_ca() {
    let h = ProxyHarness::start_default().await;
    let resp =
        send_and_read_until_close(h.addr, b"GET /ca.der HTTP/1.1\r\nHost: crosswire\r\n\r\n").await;
    let (head, body) = split_response(&resp);
    assert!(head.starts_with("HTTP/1.1 200 "), "{head}");
    assert!(
        head.contains("Content-Type: application/pkix-cert"),
        "{head}"
    );
    assert_eq!(body, h.ca.cert_der());

    let resp =
        send_and_read_until_close(h.addr, b"GET /ca.pem HTTP/1.1\r\nHost: crosswire\r\n\r\n").await;
    let (head, body) = split_response(&resp);
    assert!(
        head.contains("Content-Type: application/x-pem-file"),
        "{head}"
    );
    assert_eq!(body, h.ca.cert_pem().into_bytes());
}

#[tokio::test]
async fn ca_spki_matches() {
    let h = ProxyHarness::start_default().await;
    let resp = send_and_read_until_close(
        h.addr,
        b"GET /ca.spki HTTP/1.1\r\nHost: crosswire:8080\r\n\r\n",
    )
    .await;
    let (head, body) = split_response(&resp);
    assert!(head.starts_with("HTTP/1.1 200 "), "{head}");
    assert!(head.contains("Content-Type: text/plain"), "{head}");
    assert_eq!(
        String::from_utf8(body).unwrap().trim_end(),
        h.ca.spki_sha256_base64()
    );
}

#[tokio::test]
async fn non_loopback_bind_without_flag_fails() {
    let ca = Arc::new(crosswire_ca::Ca::generate("proxy-test").unwrap());
    let sink = Arc::new(TestSink::default());
    let cfg = ProxyConfig::new("0.0.0.0:0".parse().unwrap());
    let res = Proxy::bind(cfg, deps(&ca, sink, Arc::new(NoopHandler))).await;
    assert!(matches!(res, Err(ProxyError::NonLoopbackBind)));
}

#[tokio::test]
async fn proxy_auth_required_then_accepted() {
    let mut cfg = loopback_config();
    cfg.proxy_auth = Some(ProxyAuth::new("alice", "s3cret").unwrap());
    let h = ProxyHarness::start(cfg, Arc::new(NoopHandler)).await;

    let resp = send_and_read_until_close(
        h.addr,
        b"CONNECT example.test:443 HTTP/1.1\r\nHost: example.test:443\r\n\r\n",
    )
    .await;
    let (head, _) = split_response(&resp);
    assert!(head.starts_with("HTTP/1.1 407 "), "{head}");
    assert!(
        head.contains("Proxy-Authenticate: Basic realm=\"crosswire\"\r\n"),
        "{head}"
    );
    assert_eq!(h.counters.auth_failures.load(Ordering::Relaxed), 1);

    // Wrong password: still refused.
    let bad = base64::engine::general_purpose::STANDARD.encode("alice:wrong");
    let req = format!(
        "CONNECT example.test:443 HTTP/1.1\r\nHost: example.test:443\r\nProxy-Authorization: Basic {bad}\r\n\r\n"
    );
    let resp = send_and_read_until_close(h.addr, req.as_bytes()).await;
    assert!(resp.starts_with(b"HTTP/1.1 407 "));
    assert_eq!(h.counters.auth_failures.load(Ordering::Relaxed), 2);

    let good = base64::engine::general_purpose::STANDARD.encode("alice:s3cret");
    let req = format!(
        "CONNECT example.test:443 HTTP/1.1\r\nHost: example.test:443\r\nProxy-Authorization: Basic {good}\r\n\r\n"
    );
    let mut s = connect_raw(h.addr).await;
    s.write_all(req.as_bytes()).await.unwrap();
    let got = read_until(&mut s, b"\r\n\r\n").await;
    assert_eq!(got, b"HTTP/1.1 200 Connection established\r\n\r\n");
}

#[tokio::test]
async fn max_connections_refuses_the_next() {
    let limits = ProxyLimits::builder().max_connections(2).build();
    let h = ProxyHarness::start_with_limits(limits).await;
    let mut a = connect_raw(h.addr).await;
    let mut b = connect_raw(h.addr).await;
    // Both are held open (waiting for a request head) before the third.
    tokio::time::timeout(Duration::from_secs(5), async {
        while h.counters.accepted.load(Ordering::Relaxed) < 2 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let mut c = connect_raw(h.addr).await;
    let got = read_until_close(&mut c).await;
    assert!(got.is_empty());
    assert_eq!(
        h.counters.refused_max_connections.load(Ordering::Relaxed),
        1
    );
    assert_eq!(h.counters.accepted.load(Ordering::Relaxed), 2);

    // The first two are still alive.
    let mut buf = [0u8; 1];
    for s in [&mut a, &mut b] {
        assert!(
            tokio::time::timeout(Duration::from_millis(100), s.read(&mut buf))
                .await
                .is_err(),
            "held connection was closed"
        );
    }
}

#[tokio::test]
async fn bad_authority_is_refused() {
    let h = ProxyHarness::start_default().await;
    let resp =
        send_and_read_until_close(h.addr, b"CONNECT bad host:443 HTTP/1.1\r\nHost: x\r\n\r\n")
            .await;
    assert!(
        resp.starts_with(b"HTTP/1.1 400 "),
        "{:?}",
        String::from_utf8_lossy(&resp)
    );

    // An underscore is not a hostname character: the body says why.
    let resp =
        send_and_read_until_close(h.addr, b"CONNECT a_b.test:443 HTTP/1.1\r\nHost: x\r\n\r\n")
            .await;
    let (head, body) = split_response(&resp);
    assert!(head.starts_with("HTTP/1.1 400 "), "{head}");
    assert_eq!(
        String::from_utf8_lossy(&body),
        "Crosswire refused the request: the target is not a valid host and port."
    );

    // Valid labels, but one byte over the 253-byte name limit.
    let long = format!("{0}.{0}.{0}.{1}", "a".repeat(63), "a".repeat(62));
    assert_eq!(long.len(), 254);
    let req = format!("CONNECT {long}:443 HTTP/1.1\r\nHost: {long}:443\r\n\r\n");
    let resp = send_and_read_until_close(h.addr, req.as_bytes()).await;
    assert!(
        resp.starts_with(b"HTTP/1.1 400 "),
        "{:?}",
        String::from_utf8_lossy(&resp)
    );
}

#[tokio::test]
async fn shutdown_stops_accepting() {
    let h = ProxyHarness::start_default().await;
    let addr = h.addr;
    tokio::time::timeout(Duration::from_secs(2), h.shutdown.shutdown())
        .await
        .expect("idle proxy shuts down promptly");
    assert!(tokio::net::TcpStream::connect(addr).await.is_err());
}
