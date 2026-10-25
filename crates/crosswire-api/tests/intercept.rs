//! Intercept routes end to end: a request held by a real proxy is listed,
//! forwarded with an edit, and dropped — all driven through the API.

mod common;

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use common::{spawn_upstream, TestApp, Upstream};
use crosswire_ca::{IssuerLimits, LeafIssuer};
use crosswire_client::{DestinationGate, RustlsBackend, Transport};
use crosswire_model::ExchangeSink;
use crosswire_proxy::{Handler, Proxy, ProxyConfig, ProxyDeps, ResolveOverride};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::watch;

const OK_CLOSE: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok";

/// Starts a proxy whose handler is the app's own `Intercept`, reaching the
/// in-process upstream recorded as `u.test`.
async fn start_proxy(app: &TestApp, upstream: &Upstream) -> (SocketAddr, watch::Sender<bool>) {
    let issuer = Arc::new(LeafIssuer::new(
        Arc::clone(&app.ca),
        IssuerLimits::default(),
    ));
    let backend = Arc::new(RustlsBackend::webpki_roots());
    let mut gate = DestinationGate::new(Vec::new());
    gate.allow("127.0.0.1", upstream.port);
    let transport = Arc::new(Transport::new(backend, Arc::new(gate)));

    let up_port = upstream.port;
    let resolve: ResolveOverride = Arc::new(move |host: &str| {
        host.eq_ignore_ascii_case("u.test")
            .then_some(IpAddr::from([127, 0, 0, 1]))
    });
    let _ = up_port;

    let deps = ProxyDeps {
        resolve_override: Some(resolve),
        issuer,
        ca_der: app.ca.cert_der(),
        ca_pem: app.ca.cert_pem(),
        ca_spki: app.ca.spki_sha256_base64(),
        transport,
        sink: app.store.sink() as Arc<dyn ExchangeSink>,
        handler: Arc::clone(&app.intercept) as Arc<dyn Handler>,
        scope: Arc::clone(&app.scope),
    };
    let proxy = Proxy::bind(ProxyConfig::new("127.0.0.1:0".parse().unwrap()), deps)
        .await
        .unwrap();
    let addr = proxy.local_addr();
    let (tx, rx) = watch::channel(false);
    tokio::spawn(async move {
        let _ = proxy.run(rx).await;
    });
    (addr, tx)
}

/// Opens a connection to the proxy and sends an absolute-form GET.
async fn send_via_proxy(proxy: SocketAddr, upstream_port: u16, path: &str) -> TcpStream {
    let req = format!("GET http://u.test:{upstream_port}{path} HTTP/1.1\r\nHost: u.test\r\n\r\n");
    let mut c = TcpStream::connect(proxy).await.unwrap();
    c.write_all(req.as_bytes()).await.unwrap();
    c.flush().await.unwrap();
    c
}

async fn read_to_close(c: &mut TcpStream) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = [0u8; 8192];
    let _ = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match c.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => out.extend_from_slice(&buf[..n]),
            }
        }
    })
    .await;
    out
}

fn admin(cookie: &str) -> [(&str, &str); 3] {
    [
        ("Cookie", cookie),
        ("Sec-Fetch-Site", "same-origin"),
        ("Content-Type", "application/json"),
    ]
}

async fn wait_held(app: &TestApp, cookie: &str) -> String {
    let read = [("Cookie", cookie), ("Sec-Fetch-Site", "same-origin")];
    for _ in 0..200 {
        let resp = app.get("/api/v1/intercept", &read).await;
        let v = resp.json();
        if let Some(first) = v["held"].as_array().and_then(|a| a.first()) {
            return first["id"].as_str().unwrap().to_string();
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("no request was held in time");
}

#[tokio::test]
async fn enable_hold_forward_with_edit() {
    let upstream = spawn_upstream(OK_CLOSE.to_vec()).await;
    let app = TestApp::new().await;
    let cookie = app.session_cookie().await;
    let (proxy, _shutdown) = start_proxy(&app, &upstream).await;

    // Enable intercept through the API.
    let enable = app
        .send(
            "PUT",
            "/api/v1/intercept/enabled",
            &admin(&cookie),
            Some(b"{\"enabled\":true}"),
        )
        .await;
    assert_eq!(enable.status, 200);

    // A request through the proxy is held.
    let mut client = send_via_proxy(proxy, upstream.port, "/original").await;
    let id = wait_held(&app, &cookie).await;

    // Forward it with an edited head.
    let edited = STANDARD.encode(b"GET /edited HTTP/1.1\r\nHost: u.test\r\nX-Edited: yes\r\n\r\n");
    let fwd = app
        .send(
            "POST",
            &format!("/api/v1/intercept/held/{id}/forward"),
            &admin(&cookie),
            Some(format!("{{\"head\":\"{edited}\"}}").as_bytes()),
        )
        .await;
    assert_eq!(fwd.status, 200, "{}", fwd.body_string());

    // The client sees the response, and the upstream saw the edited bytes.
    let got = read_to_close(&mut client).await;
    assert!(String::from_utf8_lossy(&got).contains("200 OK"));
    tokio::time::sleep(Duration::from_millis(50)).await;
    let up = upstream.received.lock().unwrap().clone();
    let up = String::from_utf8_lossy(&up);
    assert!(up.contains("/edited"), "upstream got the edited path: {up}");
    assert!(!up.contains("/original"), "the original path was replaced");

    // Audit recorded the enable.
    let log = std::fs::read_to_string(app.store.dir().join("audit.jsonl")).unwrap();
    assert!(log.contains("\"intercept.enabled\""));
}

#[tokio::test]
async fn held_bytes_route_serves_original_and_404s() {
    let upstream = spawn_upstream(OK_CLOSE.to_vec()).await;
    let app = TestApp::new().await;
    let cookie = app.session_cookie().await;
    let (proxy, _shutdown) = start_proxy(&app, &upstream).await;

    let read = [
        ("Cookie", cookie.as_str()),
        ("Sec-Fetch-Site", "same-origin"),
    ];

    // An unknown id is a 404, even with a valid session.
    let unknown = app
        .get("/api/v1/intercept/held/01ARZ3NDEKTSV4RRFFQ69G5FAV/head", &read)
        .await;
    assert_eq!(unknown.status, 404, "{}", unknown.body_string());

    // Enable intercept and hold a request.
    app.send(
        "PUT",
        "/api/v1/intercept/enabled",
        &admin(&cookie),
        Some(b"{\"enabled\":true}"),
    )
    .await;
    let mut client = send_via_proxy(proxy, upstream.port, "/original").await;
    let id = wait_held(&app, &cookie).await;

    // The original head bytes come back verbatim as octet-stream.
    let head = app
        .get(&format!("/api/v1/intercept/held/{id}/head"), &read)
        .await;
    assert_eq!(head.status, 200, "{}", head.body_string());
    assert_eq!(
        head.header("content-type"),
        Some("application/octet-stream")
    );
    assert!(
        head.body_string().contains("/original"),
        "head carries the original request line"
    );
    assert_eq!(
        head.header("x-crosswire-length"),
        Some(head.body.len().to_string().as_str())
    );

    // A bodiless GET has no buffered body -> 404.
    let body = app
        .get(&format!("/api/v1/intercept/held/{id}/body"), &read)
        .await;
    assert_eq!(body.status, 404, "{}", body.body_string());

    // An unknown part -> 400.
    let bad = app
        .get(&format!("/api/v1/intercept/held/{id}/tail"), &read)
        .await;
    assert_eq!(bad.status, 400, "{}", bad.body_string());

    // Release the held request so the client connection closes cleanly.
    app.send(
        "POST",
        &format!("/api/v1/intercept/held/{id}/forward"),
        &admin(&cookie),
        None,
    )
    .await;
    let _ = read_to_close(&mut client).await;
}

#[tokio::test]
async fn hold_then_drop_closes_client() {
    let upstream = spawn_upstream(OK_CLOSE.to_vec()).await;
    let app = TestApp::new().await;
    let cookie = app.session_cookie().await;
    let (proxy, _shutdown) = start_proxy(&app, &upstream).await;

    app.send(
        "PUT",
        "/api/v1/intercept/enabled",
        &admin(&cookie),
        Some(b"{\"enabled\":true}"),
    )
    .await;

    let mut client = send_via_proxy(proxy, upstream.port, "/drop-me").await;
    let id = wait_held(&app, &cookie).await;

    let drop = app
        .send(
            "POST",
            &format!("/api/v1/intercept/held/{id}/drop"),
            &admin(&cookie),
            None,
        )
        .await;
    assert_eq!(drop.status, 200);

    let got = read_to_close(&mut client).await;
    assert!(got.is_empty(), "a dropped request closes with no response");
    assert!(
        upstream.received.lock().unwrap().is_empty(),
        "nothing reached the upstream"
    );
}
