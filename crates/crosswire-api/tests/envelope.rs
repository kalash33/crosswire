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
