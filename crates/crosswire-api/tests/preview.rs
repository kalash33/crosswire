//! The sandboxed preview route on the isolated 127.0.0.1 origin (spec §11.1):
//! origin enforcement, capability-token verification, and the deny-all CSP.
//!
//! The preview layer serves a captured response body for viewing in a sandboxed
//! iframe on a DIFFERENT browser origin (`127.0.0.1`) than the UI/API
//! (`localhost`), so the UI's session cookie never attaches. The per-exchange
//! HMAC token in the query string is the only credential.

mod common;

use bytes::Bytes;
use common::{request_message, response_message, TestApp};
use crosswire_model::{Body, Exchange, ExchangeState, ScopeState};

/// The deny-all sandbox policy the preview route must set verbatim.
const PREVIEW_CSP: &str = "sandbox; default-src 'none'; img-src data: blob:";

fn auth(cookie: &str) -> [(&str, &str); 2] {
    [("Cookie", cookie), ("Sec-Fetch-Site", "same-origin")]
}

/// Seeds one committed exchange carrying an HTML response body and returns the
/// app, a session cookie, and the exchange id.
async fn seeded() -> (TestApp, String, String) {
    let app = TestApp::new().await;
    let cookie = app.session_cookie().await;

    let mut e = Exchange::new(request_message(
        b"GET /page HTTP/1.1\r\nHost: a.test\r\n\r\n",
        Body::None,
    ));
    e.set_response(response_message(
        b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 13\r\n\r\n",
        Body::inline(Bytes::from_static(b"<html></html>")),
    ));
    e.set_scope_state(ScopeState::In);
    e.set_state(ExchangeState::Committed);
    let id = e.id().to_string();
    app.submit(e);
    app.wait_count(1).await;
    (app, cookie, id)
}

/// Mints a real preview token+path for `id` via the authenticated mint route.
async fn mint_path(app: &TestApp, cookie: &str, id: &str) -> String {
    let resp = app
        .get(
            &format!("/api/v1/exchanges/{id}/preview-token"),
            &auth(cookie),
        )
        .await;
    assert_eq!(resp.status, 200, "mint: {}", resp.body_string());
    resp.json()["path"]
        .as_str()
        .expect("a preview path")
        .to_string()
}

#[tokio::test]
async fn preview_served_on_127_with_valid_token() {
    let (app, cookie, id) = seeded().await;
    let path = mint_path(&app, &cookie, &id).await;

    // On the isolated 127.0.0.1 origin, a valid token returns the body bytes.
    let resp = app.get(&path, &[("Host", "127.0.0.1")]).await;
    assert_eq!(resp.status, 200, "{}", resp.body_string());
    assert_eq!(resp.body, b"<html></html>", "the response body is served");

    // The captured content-type is reflected so the iframe renders HTML.
    assert!(
        resp.header("content-type")
            .is_some_and(|v| v.contains("text/html")),
        "content-type reflects the captured body: {:?}",
        resp.header("content-type")
    );

    // The deny-all sandbox CSP and the two hardening headers are present.
    assert_eq!(
        resp.header("content-security-policy"),
        Some(PREVIEW_CSP),
        "the deny-all sandbox CSP is set verbatim"
    );
    assert_eq!(resp.header("referrer-policy"), Some("no-referrer"));
    assert_eq!(resp.header("x-content-type-options"), Some("nosniff"));

    // The preview origin carries no cookie and no CORS grant.
    assert!(
        resp.header("set-cookie").is_none(),
        "the preview route never sets a cookie"
    );
    assert!(
        resp.headers
            .iter()
            .all(|(k, _)| !k.starts_with("access-control-")),
        "the preview route emits no Access-Control-* header"
    );
}

#[tokio::test]
async fn preview_refused_on_localhost_host() {
    let (app, cookie, id) = seeded().await;
    let path = mint_path(&app, &cookie, &id).await;

    // The SAME valid token on the localhost origin (the harness default Host)
    // must not serve the body: origin isolation is enforced before the handler.
    let resp = app.get(&path, &[]).await;
    assert_eq!(
        resp.status,
        404,
        "a /preview request on localhost is a bare 404: {}",
        resp.body_string()
    );
    assert_ne!(
        resp.body, b"<html></html>",
        "the body must never be served on the localhost origin"
    );
}

#[tokio::test]
async fn preview_bad_token_is_403() {
    let (app, _cookie, id) = seeded().await;
    // A real exchange + body, but a forged token: 403, not the body.
    let resp = app
        .get(
            &format!("/preview/{id}?token=not-a-valid-token"),
            &[("Host", "127.0.0.1")],
        )
        .await;
    assert_eq!(resp.status, 403, "{}", resp.body_string());
    assert_ne!(resp.body, b"<html></html>");
}

#[tokio::test]
async fn preview_non_get_is_405() {
    let (app, cookie, id) = seeded().await;
    let path = mint_path(&app, &cookie, &id).await;
    let resp = app
        .send("POST", &path, &[("Host", "127.0.0.1")], None)
        .await;
    assert_eq!(resp.status, 405, "{}", resp.body_string());
}

#[tokio::test]
async fn preview_unknown_id_is_404() {
    let app = TestApp::new().await;
    // A well-formed ULID with no stored exchange/body.
    let id = "00000000000000000000000000";
    let resp = app
        .get(
            &format!("/preview/{id}?token=whatever"),
            &[("Host", "127.0.0.1")],
        )
        .await;
    assert_eq!(resp.status, 404, "{}", resp.body_string());
}
