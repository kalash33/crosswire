//! Spec §13 security gates provable without a UI (API surface).
//!
//! Each test is named after its §13 item. The server/binary gates
//! (`ca_key_never_leaves`, `audit_completeness`) live in
//! `crates/crosswire-server/tests/gates.rs`; the proxy/client gates they build
//! on are cited in `docs/security-gates.md`.

mod common;

use std::time::Duration;

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use bytes::Bytes;
use common::{request_message, response_message, spawn_upstream, Opts, TestApp, Ws};
use crosswire_model::{Body, Exchange, ExchangeState};

/// A ULID path placeholder for routes that take an id/side segment. The gate
/// rejects these requests before the handler parses the segment, so any
/// syntactically present value works.
const ZERO_ULID: &str = "00000000000000000000000000";

/// The only routes served without a credential (spec §11.2).
const UNAUTHENTICATED: &[&str] = &[
    "/api/v1/health",
    "/api/v1/openapi.json",
    "/api/v1/ca.der",
    "/api/v1/ca.pem",
    "/api/v1/session",
];

/// The WebSocket route authenticates inside its handler (it may defer to a
/// first-frame bearer), so it is never a plain `401` at the HTTP layer; it is
/// covered by [`another_localhost_port_cannot_read_any_route_or_events`] and
/// [`auth_failures_rate_limited_and_audited`].
const EVENTS: &str = "/api/v1/events";

/// Every `(path, METHOD)` the committed API advertises, read from the OpenAPI
/// document so a new route cannot silently escape the walk.
fn all_routes() -> Vec<(String, String)> {
    let doc = serde_json::to_value(crosswire_api::document()).unwrap();
    let mut out = Vec::new();
    for (path, item) in doc["paths"].as_object().expect("paths object") {
        for method in item.as_object().expect("path item").keys() {
            out.push((path.clone(), method.to_ascii_uppercase()));
        }
    }
    out
}

/// Replaces `{id}`/`{side}` templating with a concrete value.
fn concrete(path: &str) -> String {
    let mut out = String::new();
    let mut in_brace = false;
    for ch in path.chars() {
        match ch {
            '{' => {
                in_brace = true;
                out.push_str(ZERO_ULID);
            }
            '}' => in_brace = false,
            c if !in_brace => out.push(c),
            _ => {}
        }
    }
    out
}

// -- route_auth_every_route_except_the_five_is_401 --------------------------

#[tokio::test]
async fn route_auth_every_route_except_the_five_is_401() {
    // A shared app for the unauthenticated routes (they record no failures) and
    // a rolling app for the rest: each no-credential request to an
    // authenticated route is an auth failure, and ten per peer trip the rate
    // limiter, so the app is refreshed before the cap is reached.
    let shared = TestApp::new().await;
    let mut app = TestApp::new().await;
    let mut fails = 0usize;

    for (path, method) in all_routes() {
        let target = concrete(&path);
        if UNAUTHENTICATED.contains(&path.as_str()) {
            let r = shared.send(&method, &target, &[], None).await;
            assert_ne!(
                r.status, 401,
                "{method} {path} is unauthenticated and must not be 401"
            );
            continue;
        }
        if path == EVENTS {
            continue;
        }
        if fails >= 9 {
            app = TestApp::new().await;
            fails = 0;
        }
        let r = app.send(&method, &target, &[], None).await;
        assert_eq!(
            r.status, 401,
            "{method} {path} must be 401 without a credential (got {})",
            r.status
        );
        assert_eq!(
            r.json()["error"]["code"],
            "missing_credential",
            "{method} {path} returns the missing-credential envelope"
        );
        fails += 1;
    }
}

// -- route_auth_preview_is_public_but_mint_is_not ---------------------------

#[tokio::test]
async fn route_auth_preview_is_public_but_mint_is_not() {
    // The preview route (§11.1) on the isolated 127.0.0.1 origin needs no API
    // credential: the capability token in its query string is the credential, so
    // the gate runs no cookie/session check. The mint route that issues the token
    // is an ordinary `/api/v1` route and DOES need a credential. Every other
    // non-public route is covered by `route_auth_every_route_except_the_five_is_401`.
    let app = TestApp::new().await;
    let cookie = app.session_cookie().await;

    // Seed one committed exchange with an HTML response body.
    let mut e = Exchange::new(request_message(
        b"GET /p HTTP/1.1\r\nHost: a.test\r\n\r\n",
        Body::None,
    ));
    e.set_response(response_message(
        b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 13\r\n\r\n",
        Body::inline(Bytes::from_static(b"<html></html>")),
    ));
    e.set_state(ExchangeState::Committed);
    let id = e.id().to_string();
    app.submit(e);
    app.wait_count(1).await;

    // The mint route is NOT public: no credential is a 401.
    let r = app
        .get(&format!("/api/v1/exchanges/{id}/preview-token"), &[])
        .await;
    assert_eq!(r.status, 401, "the mint route must require a credential");

    // Mint a token (authenticated) and then preview it on the 127.0.0.1 origin
    // with NO API credential at all: the preview route is public.
    let path = {
        let m = app
            .get(
                &format!("/api/v1/exchanges/{id}/preview-token"),
                &[
                    ("Cookie", cookie.as_str()),
                    ("Sec-Fetch-Site", "same-origin"),
                ],
            )
            .await;
        assert_eq!(m.status, 200, "mint: {}", m.body_string());
        m.json()["path"].as_str().expect("a path").to_string()
    };
    let r = app.get(&path, &[("Host", "127.0.0.1")]).await;
    assert_ne!(
        r.status, 401,
        "the preview route must not require a credential"
    );
    assert_eq!(
        r.status,
        200,
        "the preview route is served with only the capability token: {}",
        r.body_string()
    );
}

// -- another_localhost_port_cannot_read_any_route_or_events -----------------

#[tokio::test]
async fn another_localhost_port_cannot_read_any_route_or_events() {
    // A request from a page on a different localhost port carries the victim's
    // cookie, an `Origin` of that other port and no `Sec-Fetch-Site`. Every
    // authenticated route, and the WebSocket upgrade, must refuse it with 403.
    let foreign = "http://localhost:9999";
    let mut app = TestApp::new().await;
    let mut cookie = app.session_cookie().await;
    let mut fails = 0usize;

    for (path, method) in all_routes() {
        if UNAUTHENTICATED.contains(&path.as_str()) || path == EVENTS {
            continue;
        }
        if fails >= 9 {
            app = TestApp::new().await;
            cookie = app.session_cookie().await;
            fails = 0;
        }
        let r = app
            .send(
                &method,
                &concrete(&path),
                &[("Cookie", &cookie), ("Origin", foreign)],
                None,
            )
            .await;
        assert_eq!(
            r.status, 403,
            "{method} {path} must be 403 for a foreign-origin cookie (got {})",
            r.status
        );
        assert_eq!(
            r.json()["error"]["code"],
            "origin_forbidden",
            "{method} {path} returns the origin-forbidden envelope"
        );
        fails += 1;
    }

    // The WebSocket upgrade is refused the same way (never upgraded to 101).
    let err = Ws::connect(
        app.addr,
        app.port,
        EVENTS,
        &[("Cookie", &cookie), ("Origin", foreign)],
    )
    .await
    .err();
    assert_eq!(err, Some(403), "the events upgrade must be 403, not 101");
}

// -- host_421 ---------------------------------------------------------------

#[tokio::test]
async fn host_421() {
    // A non-loopback Host is refused on every route (the check runs first, so
    // it records no auth failure and one app serves the whole walk).
    let app = TestApp::new().await;
    for (path, method) in all_routes() {
        let r = app
            .send(&method, &concrete(&path), &[("Host", "evil.test")], None)
            .await;
        assert_eq!(
            r.status, 421,
            "{method} {path} must be 421 for a non-loopback Host (got {})",
            r.status
        );
        assert_eq!(r.json()["error"]["code"], "bad_host");
    }

    // The WebSocket upgrade is refused by the same Host check.
    let err = Ws::connect(app.addr, app.port, EVENTS, &[("Host", "evil.test")])
        .await
        .err();
    assert_eq!(
        err,
        Some(421),
        "the events upgrade must be 421 for a bad Host"
    );
}

// -- scope_matrix_including_empty_scope -------------------------------------

#[tokio::test]
async fn scope_matrix_including_empty_scope() {
    // Drives the repeater (through the API) across the four scope verdicts an
    // empty, including and excluding scope produce, and proves strict mode
    // refuses an out-of-scope send. The history-side matrix (In/Out/Empty and
    // HostMismatch recorded on proxy exchanges) is proven by
    // `crosswire-proxy/tests/intercept.rs::scope_state_is_recorded_on_history_exchanges`.
    let upstream = spawn_upstream(
        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec(),
    )
    .await;
    let app = TestApp::with(Opts {
        allow: vec![("127.0.0.1".to_string(), upstream.port)],
        ..Opts::default()
    })
    .await;
    let cookie = app.session_cookie().await;
    let admin = [
        ("Cookie", cookie.as_str()),
        ("Sec-Fetch-Site", "same-origin"),
        ("Content-Type", "application/json"),
    ];

    let raw = STANDARD.encode("GET / HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n");
    let send_body = |allow_oos: bool| {
        format!(
            "{{\"host\":\"127.0.0.1\",\"port\":{},\"tls\":false,\"raw\":\"{raw}\",\"allow_out_of_scope\":{allow_oos}}}",
            upstream.port
        )
    };
    let audit_has = |needle: &str| {
        std::fs::read_to_string(app.store.dir().join("audit.jsonl"))
            .unwrap_or_default()
            .contains(needle)
    };

    // 1. Empty scope: the send is Empty and raises no out-of-scope warning.
    let r = app
        .send(
            "POST",
            "/api/v1/repeater/send",
            &admin,
            Some(send_body(true).as_bytes()),
        )
        .await;
    assert_eq!(r.status, 200, "empty-scope send: {}", r.body_string());
    assert_eq!(r.json()["scope_state"], "empty");
    assert!(
        !audit_has("repeater.send.out_of_scope"),
        "an empty-scope send must not be audited out-of-scope"
    );

    // 2. Scope includes the target: In.
    let r = app
        .send(
            "PUT",
            "/api/v1/scope",
            &admin,
            Some(br#"{"include":[{"host":{"type":"ip","value":"127.0.0.1"}}],"exclude":[]}"#),
        )
        .await;
    assert_eq!(r.status, 200, "put in-scope: {}", r.body_string());
    let r = app
        .send(
            "POST",
            "/api/v1/repeater/send",
            &admin,
            Some(send_body(true).as_bytes()),
        )
        .await;
    assert_eq!(r.status, 200, "in-scope send: {}", r.body_string());
    assert_eq!(r.json()["scope_state"], "in");

    // 3. Scope excludes the target: Out, allowed, and audited out-of-scope.
    let r = app
        .send(
            "PUT",
            "/api/v1/scope",
            &admin,
            Some(br#"{"include":[{"host":{"type":"exact","value":"other.test"}}],"exclude":[]}"#),
        )
        .await;
    assert_eq!(r.status, 200, "put out-of-scope: {}", r.body_string());
    let r = app
        .send(
            "POST",
            "/api/v1/repeater/send",
            &admin,
            Some(send_body(true).as_bytes()),
        )
        .await;
    assert_eq!(
        r.status,
        200,
        "out-of-scope allowed send: {}",
        r.body_string()
    );
    assert_eq!(r.json()["scope_state"], "out");
    assert!(
        audit_has("repeater.send.out_of_scope"),
        "an allowed out-of-scope send is audited"
    );

    // 4a. Out of scope and not allowed: refused.
    let r = app
        .send(
            "POST",
            "/api/v1/repeater/send",
            &admin,
            Some(send_body(false).as_bytes()),
        )
        .await;
    assert_eq!(r.status, 403, "unallowed out-of-scope send is refused");
    assert_eq!(r.json()["error"]["code"], "out_of_scope");

    // 4b. Strict mode refuses even an allowed out-of-scope send.
    let r = app
        .send(
            "PUT",
            "/api/v1/settings/enforce_scope_strictly",
            &admin,
            Some(br#"{"enabled":true}"#),
        )
        .await;
    assert_eq!(r.status, 200, "enable strict: {}", r.body_string());
    let r = app
        .send(
            "POST",
            "/api/v1/repeater/send",
            &admin,
            Some(send_body(true).as_bytes()),
        )
        .await;
    assert_eq!(r.status, 403, "strict mode refuses an out-of-scope send");
    assert_eq!(r.json()["error"]["code"], "out_of_scope");
}

// -- idle_egress_zero_connections ------------------------------------------

#[tokio::test]
async fn idle_egress_zero_connections() {
    // With the API open and the event stream connected, an idle sidecar must
    // make zero upstream connections and zero DNS lookups: nothing phones home.
    let app = TestApp::new().await;
    let cookie = app.session_cookie().await;
    let secret = app.make_token("events-reader", &["read"], &cookie).await;
    let auth = format!("Bearer {secret}");

    assert_eq!(app.transport.connect_count(), 0, "no connects before idle");
    assert_eq!(app.transport.dns_count(), 0, "no DNS lookups before idle");

    // Connect the event stream (authenticated via the bearer header) and hold
    // it open for the idle window.
    let mut ws = Ws::connect(
        app.addr,
        app.port,
        "/api/v1/events",
        &[("Authorization", &auth)],
    )
    .await
    .expect("the events stream upgrades");

    tokio::time::sleep(Duration::from_secs(10)).await;

    // The stream is still live (no frame is expected while idle, so a short
    // read times out and returns None without the socket having closed).
    let _ = ws.recv_text(Duration::from_millis(50)).await;

    assert_eq!(
        app.transport.connect_count(),
        0,
        "an idle sidecar makes no upstream connection"
    );
    assert_eq!(
        app.transport.dns_count(),
        0,
        "an idle sidecar performs no DNS lookup"
    );
}

// -- auth_failures_rate_limited_and_audited --------------------------------

#[tokio::test]
async fn auth_failures_rate_limited_and_audited() {
    // Ten bad first-frame bearer tokens over the WebSocket exhaust the per-peer
    // cap; the eleventh attempt is rate-limited before the upgrade, and every
    // failure is written to the audit log. This exercises the first-frame path
    // the gate closed in Task 6.
    let app = TestApp::new().await;

    for i in 0..10 {
        let mut ws = Ws::connect(app.addr, app.port, "/api/v1/events", &[])
            .await
            .unwrap_or_else(|s| panic!("attempt {i} should upgrade, got {s}"));
        ws.send_text("{\"bearer\":\"cwt_deadbeefdeadbeef\"}").await;
        // The server records the failure, audits it, then closes the socket.
        let closed = ws.recv_text(Duration::from_secs(2)).await;
        assert!(closed.is_none(), "attempt {i} is closed after a bad token");
    }

    // The eleventh attempt (still credential-less at the header layer) is
    // refused before the upgrade with 429.
    let err = Ws::connect(app.addr, app.port, "/api/v1/events", &[])
        .await
        .err();
    assert_eq!(err, Some(429), "the eleventh attempt is rate limited");

    let log = std::fs::read_to_string(app.store.dir().join("audit.jsonl")).unwrap();
    let fails = log
        .lines()
        .filter(|l| l.contains("\"auth.fail\"") && l.contains("/api/v1/events"))
        .count();
    assert!(
        fails >= 10,
        "every first-frame failure is audited (got {fails})"
    );
}
