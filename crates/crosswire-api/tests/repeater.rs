//! Repeater routes: raw and assisted sends over a real upstream, and the
//! draft round trip.

mod common;

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use common::{spawn_upstream, Opts, TestApp};

const OK_CLOSE: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok";

async fn app_with_upstream() -> (TestApp, common::Upstream, String) {
    let upstream = spawn_upstream(OK_CLOSE.to_vec()).await;
    let app = TestApp::with(Opts {
        allow: vec![("127.0.0.1".to_string(), upstream.port)],
        ..Opts::default()
    })
    .await;
    let cookie = app.session_cookie().await;
    (app, upstream, cookie)
}

fn send_headers(cookie: &str) -> [(&str, &str); 3] {
    [
        ("Cookie", cookie),
        ("Sec-Fetch-Site", "same-origin"),
        ("Content-Type", "application/json"),
    ]
}

#[tokio::test]
async fn raw_send_reaches_upstream() {
    let (app, upstream, cookie) = app_with_upstream().await;
    let raw = STANDARD.encode(b"GET /raw HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n");
    let body = format!(
        "{{\"host\":\"127.0.0.1\",\"port\":{},\"raw\":\"{raw}\",\"mode\":\"raw\"}}",
        upstream.port
    );
    let resp = app
        .send(
            "POST",
            "/api/v1/repeater/send",
            &send_headers(&cookie),
            Some(body.as_bytes()),
        )
        .await;
    assert_eq!(resp.status, 200, "{}", resp.body_string());
    let v = resp.json();
    assert!(v["exchange_id"].as_str().is_some(), "an exchange id");

    // The upstream saw the exact raw bytes.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let got = upstream.received.lock().unwrap().clone();
    assert!(
        got.windows(16).any(|w| w == b"GET /raw HTTP/1."),
        "the upstream received the raw request"
    );
}

#[tokio::test]
async fn assisted_send_fixes_framing() {
    let (app, upstream, cookie) = app_with_upstream().await;
    // A body with a wrong Content-Length; assisted mode recomputes it.
    let raw =
        STANDARD.encode(b"POST /a HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 999\r\n\r\nhi");
    let body = format!(
        "{{\"host\":\"127.0.0.1\",\"port\":{},\"raw\":\"{raw}\",\"mode\":\"assisted\"}}",
        upstream.port
    );
    let resp = app
        .send(
            "POST",
            "/api/v1/repeater/send",
            &send_headers(&cookie),
            Some(body.as_bytes()),
        )
        .await;
    assert_eq!(resp.status, 200, "{}", resp.body_string());
    let v = resp.json();
    assert!(v["changes"].is_array(), "changes are reported");
    assert!(
        !v["changes"].as_array().unwrap().is_empty(),
        "assisted mode recorded at least one edit"
    );
}

#[tokio::test]
async fn draft_round_trip() {
    let app = TestApp::new().await;
    let cookie = app.session_cookie().await;
    let h = send_headers(&cookie);
    let read = [
        ("Cookie", cookie.as_str()),
        ("Sec-Fetch-Site", "same-origin"),
    ];
    let id = "01AN4Z07BY79KA1307SR9X4MV3";

    // Save.
    let put = app
        .send(
            "PUT",
            &format!("/api/v1/repeater/tabs/{id}"),
            &h,
            Some(b"{\"request\":\"GET / HTTP/1.1\"}"),
        )
        .await;
    assert_eq!(put.status, 200, "{}", put.body_string());

    // Fetch.
    let got = app.get(&format!("/api/v1/repeater/tabs/{id}"), &read).await;
    assert_eq!(got.status, 200);
    assert_eq!(got.json()["request"], "GET / HTTP/1.1");

    // List.
    let list = app.get("/api/v1/repeater/tabs", &read).await;
    assert_eq!(list.status, 200);
    assert!(!list.json().as_array().unwrap().is_empty());

    // Delete.
    let del = app
        .send(
            "DELETE",
            &format!("/api/v1/repeater/tabs/{id}"),
            &read,
            None,
        )
        .await;
    assert_eq!(del.status, 200);
    let after = app.get(&format!("/api/v1/repeater/tabs/{id}"), &read).await;
    assert_eq!(after.status, 404);
}
