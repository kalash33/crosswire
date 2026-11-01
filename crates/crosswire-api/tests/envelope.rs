//! The version envelope: the `Crosswire-Schema-Version` header on every gated
//! response and the version fields of the health body.

mod common;

use common::{Opts, TestApp};

#[tokio::test]
async fn schema_version_header_on_every_gated_response() {
    let app = TestApp::new().await;

    let ok = app.get("/api/v1/health", &[]).await;
    assert_eq!(ok.status, 200);
    assert_eq!(ok.header("crosswire-schema-version"), Some("2"));

    // A credential-less protected route: the gate's own 401.
    let denied = app.get("/api/v1/exchanges", &[]).await;
    assert_eq!(denied.status, 401);
    assert_eq!(denied.header("crosswire-schema-version"), Some("2"));

    // A non-loopback Host: the gate's own 421.
    let bad_host = app.get("/api/v1/health", &[("Host", "evil.test")]).await;
    assert_eq!(bad_host.status, 421);
    assert_eq!(bad_host.header("crosswire-schema-version"), Some("2"));

    // The gate's 404 for `/preview/` on a non-preview Host.
    let preview = app.get("/preview/x", &[]).await;
    assert_eq!(preview.status, 404);
    assert_eq!(preview.header("crosswire-schema-version"), Some("2"));
}

#[tokio::test]
async fn health_reports_schema_and_format_versions() {
    let app = TestApp::new().await;
    let r = app.get("/api/v1/health", &[]).await;
    assert_eq!(r.status, 200);
    let j = r.json();
    assert_eq!(j["status"], "ok");
    assert_eq!(j["schema_version"], 2);
    assert_eq!(j["format_version"], 6);
}

#[tokio::test]
async fn health_reports_proxy_addr_when_configured() {
    let unconfigured = TestApp::new().await;
    let j = unconfigured.get("/api/v1/health", &[]).await.json();
    assert!(
        j.get("proxy_addr").is_none(),
        "absent when unconfigured: {j}"
    );

    for addr in ["127.0.0.1:8081", "[::1]:9090"] {
        let proxy: std::net::SocketAddr = addr.parse().unwrap();
        let app = TestApp::with(Opts {
            proxy_addr: Some(proxy),
            ..Opts::default()
        })
        .await;
        let j = app.get("/api/v1/health", &[]).await.json();
        assert_eq!(j["proxy_addr"], addr);
    }
}

#[tokio::test]
async fn health_reports_the_project_when_configured() {
    let unconfigured = TestApp::new().await;
    let j = unconfigured.get("/api/v1/health", &[]).await.json();
    assert!(j.get("project").is_none(), "absent when unconfigured: {j}");

    let app = TestApp::with(Opts {
        project: Some(crosswire_api::ProjectInfo {
            name: "acme <script>".into(),
            path: "/work/acme <script>".into(),
        }),
        ..Opts::default()
    })
    .await;
    let j = app.get("/api/v1/health", &[]).await.json();
    assert_eq!(
        j["project"],
        serde_json::json!({"name": "acme <script>", "path": "/work/acme <script>"})
    );
}

/// `project` names a filesystem path (user name, engagement name), so it is
/// reported only to loopback peers: a non-loopback `--api-bind` must not hand
/// it to the LAN. A request with no peer info (an embedder without
/// `ConnectInfo`) is treated as loopback, as the gate does.
#[tokio::test]
async fn health_reports_the_project_only_to_loopback_peers() {
    use axum::extract::ConnectInfo;
    use tower::ServiceExt;

    let app = TestApp::with(Opts {
        project: Some(crosswire_api::ProjectInfo {
            name: "acme".into(),
            path: "/home/u/engagements/acme".into(),
        }),
        proxy_addr: Some("127.0.0.1:8080".parse().unwrap()),
        ..Opts::default()
    })
    .await;

    let health = |peer: Option<&str>| {
        let router = crosswire_api::router(app.state.clone());
        let mut req = http::Request::get("/api/v1/health")
            .header("host", "localhost")
            .body(axum::body::Body::empty())
            .unwrap();
        if let Some(p) = peer {
            let addr: std::net::SocketAddr = p.parse().unwrap();
            req.extensions_mut().insert(ConnectInfo(addr));
        }
        async move {
            let resp = router.oneshot(req).await.unwrap();
            assert_eq!(resp.status(), 200);
            let body = axum::body::to_bytes(resp.into_body(), 1 << 20)
                .await
                .unwrap();
            serde_json::from_slice::<serde_json::Value>(&body).unwrap()
        }
    };

    for peer in [
        "192.0.2.7:50000",
        "[2001:db8::1]:50000",
        "10.0.0.5:1",
        "[::ffff:192.0.2.7]:1",
    ] {
        let j = health(Some(peer)).await;
        assert!(j.get("project").is_none(), "{peer}: {j}");
        // proxy_addr names a bind address, not a path: still reported.
        assert_eq!(j["proxy_addr"], "127.0.0.1:8080", "{peer}");
    }
    for peer in [
        Some("127.0.0.1:50000"),
        Some("[::1]:50000"),
        Some("[::ffff:127.0.0.1]:1"),
        None,
    ] {
        let j = health(peer).await;
        assert_eq!(j["project"]["path"], "/home/u/engagements/acme", "{peer:?}");
    }
}
