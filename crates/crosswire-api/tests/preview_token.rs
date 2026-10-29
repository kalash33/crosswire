//! The preview capability-token mint route: scope gate, response shape, and
//! the `preview.mint` audit row (spec §11.1).

mod common;

use bytes::Bytes;
use common::{request_message, response_message, TestApp};
use crosswire_model::{Body, Exchange, ExchangeState, ScopeState};

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

#[tokio::test]
async fn mint_requires_read_scope() {
    let (app, _cookie, id) = seeded().await;
    // No credential at all: the gate rejects before the handler runs.
    let resp = app
        .get(&format!("/api/v1/exchanges/{id}/preview-token"), &[])
        .await;
    assert_eq!(resp.status, 401, "{}", resp.body_string());
}

#[tokio::test]
async fn mint_returns_token_and_path_and_is_audited() {
    let (app, cookie, id) = seeded().await;
    let resp = app
        .get(
            &format!("/api/v1/exchanges/{id}/preview-token"),
            &auth(&cookie),
        )
        .await;
    assert_eq!(resp.status, 200, "{}", resp.body_string());

    let v = resp.json();
    let token = v["token"].as_str().expect("a token");
    assert!(!token.is_empty(), "the token is non-empty");
    assert_eq!(v["expires_in"], 60, "ttl is 60 seconds");
    assert_eq!(
        v["path"].as_str().expect("a path"),
        format!("/preview/{id}?token={token}"),
        "the path carries the token in its query string"
    );

    // The mint was audited as `preview.mint` against the exchange id.
    let log = std::fs::read_to_string(app.store.dir().join("audit.jsonl")).unwrap();
    assert!(
        log.lines().any(|l| {
            let row: serde_json::Value = serde_json::from_str(l).unwrap_or_default();
            row["action"] == "preview.mint" && row["target"] == id && row["result"] == "ok"
        }),
        "a preview.mint audit row was written for {id}; log:\n{log}"
    );
}

#[tokio::test]
async fn mint_404_without_a_body() {
    let app = TestApp::new().await;
    let cookie = app.session_cookie().await;
    // An exchange with no response body cannot be previewed.
    let mut e = Exchange::new(request_message(
        b"GET /empty HTTP/1.1\r\nHost: a.test\r\n\r\n",
        Body::None,
    ));
    e.set_response(response_message(
        b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n",
        Body::None,
    ));
    e.set_state(ExchangeState::Committed);
    let id = e.id().to_string();
    app.submit(e);
    app.wait_count(1).await;

    let resp = app
        .get(
            &format!("/api/v1/exchanges/{id}/preview-token"),
            &auth(&cookie),
        )
        .await;
    assert_eq!(resp.status, 404, "{}", resp.body_string());
}
