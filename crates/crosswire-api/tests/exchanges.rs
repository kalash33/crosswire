//! Exchange read routes: filters, keyset paging, fetch, body truncation
//! header, and patching tags and notes.

mod common;

use bytes::Bytes;
use common::{request_message, response_message, TestApp};
use crosswire_model::{Body, Exchange, ExchangeState, Namespace, NamespaceKind, ScopeState};

fn req(raw: &str) -> Exchange {
    Exchange::new(request_message(raw.as_bytes(), Body::None))
}

/// The headers an authenticated read request carries.
fn auth(cookie: &str) -> [(&str, &str); 2] {
    [("Cookie", cookie), ("Sec-Fetch-Site", "same-origin")]
}

async fn seeded() -> (TestApp, String) {
    let app = TestApp::new().await;
    let cookie = app.session_cookie().await;

    // ex1: GET a.test /one -> 200, history, in-scope, tag alpha.
    let mut e1 = req("GET /one HTTP/1.1\r\nHost: a.test\r\n\r\n");
    e1.set_response(response_message(
        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n",
        Body::inline(Bytes::from_static(b"ok")),
    ));
    e1.set_scope_state(ScopeState::In);
    e1.set_state(ExchangeState::Committed);
    e1.add_tag("alpha");
    app.submit(e1);

    // ex2: POST b.test /two -> 404, repeater, out-of-scope, tag beta.
    let mut e2 = req("POST /two HTTP/1.1\r\nHost: b.test\r\n\r\n");
    e2.set_response(response_message(
        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n",
        Body::None,
    ));
    e2.set_scope_state(ScopeState::Out);
    e2.set_namespace(Namespace::new(NamespaceKind::Repeater, None));
    e2.set_state(ExchangeState::Error);
    e2.add_tag("beta");
    app.submit(e2);

    // ex3: GET a.test /three -> 500, with a truncated request body.
    let mut e3 = Exchange::new(request_message(
        b"GET /three HTTP/1.1\r\nHost: a.test\r\n\r\n",
        Body::Inline {
            bytes: Bytes::from_static(b"abc"),
            truncated_at: Some(3),
            capture_dropped: false,
            relay: None,
        },
    ));
    e3.set_response(response_message(
        b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n",
        Body::None,
    ));
    e3.set_scope_state(ScopeState::In);
    e3.set_state(ExchangeState::Committed);
    app.submit(e3);

    app.wait_count(3).await;
    (app, cookie)
}

#[tokio::test]
async fn list_filters_and_paging() {
    let (app, cookie) = seeded().await;
    let h = auth(&cookie);

    // All three.
    let all = app.get("/api/v1/exchanges", &h).await;
    assert_eq!(all.status, 200);
    assert_eq!(all.json()["items"].as_array().unwrap().len(), 3);

    // method filter.
    let posts = app.get("/api/v1/exchanges?method=POST", &h).await;
    assert_eq!(posts.json()["items"].as_array().unwrap().len(), 1);

    // exact host.
    let a = app.get("/api/v1/exchanges?host=a.test", &h).await;
    assert_eq!(a.json()["items"].as_array().unwrap().len(), 2);

    // host substring.
    let b = app.get("/api/v1/exchanges?host_contains=b.te", &h).await;
    assert_eq!(b.json()["items"].as_array().unwrap().len(), 1);

    // status range.
    let errs = app
        .get("/api/v1/exchanges?status_min=400&status_max=599", &h)
        .await;
    assert_eq!(errs.json()["items"].as_array().unwrap().len(), 2);

    // namespace.
    let rep = app.get("/api/v1/exchanges?namespace=repeater", &h).await;
    assert_eq!(rep.json()["items"].as_array().unwrap().len(), 1);

    // state.
    let errored = app.get("/api/v1/exchanges?state=error", &h).await;
    assert_eq!(errored.json()["items"].as_array().unwrap().len(), 1);

    // scope_state.
    let out = app.get("/api/v1/exchanges?scope_state=out", &h).await;
    assert_eq!(out.json()["items"].as_array().unwrap().len(), 1);

    // tag.
    let alpha = app.get("/api/v1/exchanges?tag=alpha", &h).await;
    assert_eq!(alpha.json()["items"].as_array().unwrap().len(), 1);

    // keyset paging: limit 1, then follow next_cursor.
    let p1 = app.get("/api/v1/exchanges?limit=1", &h).await;
    let v1 = p1.json();
    assert_eq!(v1["items"].as_array().unwrap().len(), 1);
    let cursor = v1["next_cursor"].as_str().expect("a cursor").to_string();
    let p2 = app
        .get(&format!("/api/v1/exchanges?limit=1&after={cursor}"), &h)
        .await;
    let v2 = p2.json();
    assert_eq!(v2["items"].as_array().unwrap().len(), 1);
    assert_ne!(v1["items"][0]["id"], v2["items"][0]["id"], "a new row");
}

#[tokio::test]
async fn get_one_and_body_with_truncation_header() {
    let (app, cookie) = seeded().await;
    let h = auth(&cookie);

    // Find the /three exchange id.
    let listed = app.get("/api/v1/exchanges?host=a.test", &h).await;
    let items = listed.json();
    let three = items["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["path"].as_str() == Some("/three"))
        .unwrap()
        .clone();
    let id = three["id"].as_str().unwrap().to_string();

    // GET the exchange.
    let got = app.get(&format!("/api/v1/exchanges/{id}"), &h).await;
    assert_eq!(got.status, 200);

    // The request body carries the truncation header.
    let body = app
        .get(&format!("/api/v1/exchanges/{id}/body/req"), &h)
        .await;
    assert_eq!(body.status, 200);
    assert_eq!(body.body, b"abc");
    assert_eq!(body.header("x-crosswire-length"), Some("3"));
    assert_eq!(body.header("x-crosswire-truncated-at"), Some("3"));
    assert_eq!(
        body.header("content-type"),
        Some("application/octet-stream")
    );

    // A missing exchange is 404.
    let missing = app
        .get("/api/v1/exchanges/00000000000000000000000000", &h)
        .await;
    assert_eq!(missing.status, 404);
}

#[tokio::test]
async fn patch_tags_and_notes() {
    let (app, cookie) = seeded().await;
    let h = auth(&cookie);
    let send_h = [
        ("Cookie", cookie.as_str()),
        ("Sec-Fetch-Site", "same-origin"),
        ("Content-Type", "application/json"),
    ];

    let all = app.get("/api/v1/exchanges", &h).await;
    let id = all.json()["items"][0]["id"].as_str().unwrap().to_string();

    let patch = app
        .send(
            "PATCH",
            &format!("/api/v1/exchanges/{id}"),
            &send_h,
            Some(b"{\"tags\":[\"reviewed\"],\"notes\":\"looks ok\"}"),
        )
        .await;
    assert_eq!(patch.status, 200);

    let got = app.get(&format!("/api/v1/exchanges/{id}"), &h).await;
    let v = got.json();
    assert_eq!(v["tags"][0], "reviewed");
    assert_eq!(v["notes"], "looks ok");

    // Patching a missing exchange is 404.
    let missing = app
        .send(
            "PATCH",
            "/api/v1/exchanges/00000000000000000000000000",
            &send_h,
            Some(b"{\"notes\":\"x\"}"),
        )
        .await;
    assert_eq!(missing.status, 404);
}

#[tokio::test]
async fn counts_and_markers() {
    let (app, cookie) = seeded().await;
    let h = auth(&cookie);

    let counts = app.get("/api/v1/counts", &h).await;
    assert_eq!(counts.status, 200);
    let v = counts.json();
    assert_eq!(v["exchanges"].as_u64(), Some(3));
    assert_eq!(v["repeater"].as_u64(), Some(1));
    assert_eq!(v["history"].as_u64(), Some(2));

    let markers = app.get("/api/v1/markers", &h).await;
    assert_eq!(markers.status, 200);
    assert!(markers.json()["items"].is_array());
}
