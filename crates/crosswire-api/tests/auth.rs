//! Authentication, authorisation, cookie isolation, the Host check, rate
//! limiting and the no-CORS rule.

mod common;

use std::time::Duration;

use common::{Opts, TestApp};

const ZERO_ULID: &str = "00000000000000000000000000";

#[tokio::test]
async fn bootstrap_is_single_use_and_expires() {
    let app = TestApp::with(Opts {
        bootstrap_ttl: Duration::from_millis(200),
        ..Opts::default()
    })
    .await;

    // A fresh token works once.
    let token = app.mint_bootstrap();
    let body = format!("{{\"bootstrap\":\"{token}\"}}");
    let ok = app
        .send(
            "POST",
            "/api/v1/session",
            &[("Content-Type", "application/json")],
            Some(body.as_bytes()),
        )
        .await;
    assert_eq!(ok.status, 200, "first use succeeds");
    assert!(ok.header("set-cookie").unwrap().contains("cw_sid_"));
    let v = ok.json();
    assert_eq!(v["expires_in"].as_u64(), Some(30 * 60));

    // Reusing the same token fails (single use).
    let reuse = app
        .send(
            "POST",
            "/api/v1/session",
            &[("Content-Type", "application/json")],
            Some(body.as_bytes()),
        )
        .await;
    assert_eq!(reuse.status, 401, "a reused token is rejected");
    assert_eq!(reuse.json()["error"]["code"], "bootstrap_invalid");

    // A token that has expired fails.
    let token2 = app.mint_bootstrap();
    tokio::time::sleep(Duration::from_millis(250)).await;
    let body2 = format!("{{\"bootstrap\":\"{token2}\"}}");
    let expired = app
        .send(
            "POST",
            "/api/v1/session",
            &[("Content-Type", "application/json")],
            Some(body2.as_bytes()),
        )
        .await;
    assert_eq!(expired.status, 401, "an expired token is rejected");
}

#[tokio::test]
async fn bootstrap_rate_limited_after_10_failures_and_audited() {
    let app = TestApp::new().await;
    let bad = "{\"bootstrap\":\"not-the-token\"}";
    for _ in 0..10 {
        let r = app
            .send(
                "POST",
                "/api/v1/session",
                &[("Content-Type", "application/json")],
                Some(bad.as_bytes()),
            )
            .await;
        assert_eq!(r.status, 401, "each wrong attempt is 401");
    }
    let limited = app
        .send(
            "POST",
            "/api/v1/session",
            &[("Content-Type", "application/json")],
            Some(bad.as_bytes()),
        )
        .await;
    assert_eq!(limited.status, 429, "the 11th attempt is rate limited");
    assert_eq!(limited.json()["error"]["code"], "rate_limited");

    // The failures were audited.
    let log = std::fs::read_to_string(app.store.dir().join("audit.jsonl")).unwrap();
    let fails = log
        .lines()
        .filter(|l| l.contains("\"session.bootstrap\"") && l.contains("\"fail\""))
        .count();
    assert!(fails >= 10, "bootstrap failures are audited (got {fails})");
}

#[tokio::test]
async fn cookie_without_sec_fetch_site_is_403() {
    let app = TestApp::new().await;
    let cookie = app.session_cookie().await;

    // A cookie with neither Sec-Fetch-Site nor Origin is refused.
    let bare = app.get("/api/v1/counts", &[("Cookie", &cookie)]).await;
    assert_eq!(bare.status, 403, "cookie without same-origin proof is 403");
    assert_eq!(bare.json()["error"]["code"], "origin_forbidden");

    // Sec-Fetch-Site: same-origin is accepted.
    let same = app
        .get(
            "/api/v1/counts",
            &[("Cookie", &cookie), ("Sec-Fetch-Site", "same-origin")],
        )
        .await;
    assert_eq!(same.status, 200, "same-origin is accepted");

    // An exact-origin header is accepted too.
    let origin = format!("http://localhost:{}", app.port);
    let with_origin = app
        .get(
            "/api/v1/counts",
            &[("Cookie", &cookie), ("Origin", &origin)],
        )
        .await;
    assert_eq!(with_origin.status, 200, "exact Origin is accepted");

    // A foreign Origin is refused.
    let foreign = app
        .get(
            "/api/v1/counts",
            &[("Cookie", &cookie), ("Origin", "http://evil.test")],
        )
        .await;
    assert_eq!(foreign.status, 403, "a foreign Origin is refused");
}

#[tokio::test]
async fn host_other_than_localhost_is_421() {
    let app = TestApp::new().await;
    let bad = app.get("/api/v1/health", &[("Host", "evil.test")]).await;
    assert_eq!(bad.status, 421, "a non-loopback Host is 421");
    assert_eq!(bad.json()["error"]["code"], "bad_host");

    // 127.0.0.1 and localhost (with a port) are accepted.
    let ok = app
        .get("/api/v1/health", &[("Host", "127.0.0.1:1234")])
        .await;
    assert_eq!(ok.status, 200);
}

#[tokio::test]
async fn read_token_cannot_mutate() {
    let app = TestApp::new().await;
    let cookie = app.session_cookie().await;
    let secret = app.make_token("reader", &["read"], &cookie).await;
    let auth = format!("Bearer {secret}");

    // A read token can read.
    let read = app.get("/api/v1/counts", &[("Authorization", &auth)]).await;
    assert_eq!(read.status, 200, "a read token can read");

    // Every send/admin route rejects it with 403 insufficient_scope.
    let mutations: Vec<(&str, String, &str)> = vec![
        ("POST", "/api/v1/repeater/send".into(), "{\"host\":\"x\",\"port\":80,\"raw\":\"aGk=\"}"),
        ("PUT", format!("/api/v1/repeater/tabs/{ZERO_ULID}"), "{}"),
        ("DELETE", format!("/api/v1/repeater/tabs/{ZERO_ULID}"), ""),
        ("POST", format!("/api/v1/intercept/held/{ZERO_ULID}/forward"), "{}"),
        ("POST", format!("/api/v1/intercept/held/{ZERO_ULID}/drop"), ""),
        ("POST", "/api/v1/intercept/forward-all".into(), ""),
        ("PATCH", format!("/api/v1/exchanges/{ZERO_ULID}"), "{}"),
        ("PUT", "/api/v1/scope".into(), "{}"),
        ("POST", "/api/v1/scope/import".into(), "{\"text\":\"\"}"),
        ("PUT", "/api/v1/intercept/enabled".into(), "{\"enabled\":true}"),
        (
            "PUT",
            "/api/v1/intercept/rules".into(),
            "{\"skip_extensions\":[],\"skip_out_of_scope\":true,\"overflow\":\"forward-newest\",\"on_timeout\":\"forward\"}",
        ),
        ("PUT", "/api/v1/settings/enforce_scope_strictly".into(), "{\"enabled\":true}"),
        ("POST", "/api/v1/tokens".into(), "{\"name\":\"x\",\"scopes\":[\"read\"]}"),
        ("DELETE", format!("/api/v1/tokens/{ZERO_ULID}"), ""),
    ];
    for (method, path, body) in mutations {
        let b = if body.is_empty() {
            None
        } else {
            Some(body.as_bytes())
        };
        let resp = app
            .send(
                method,
                &path,
                &[
                    ("Authorization", &auth),
                    ("Content-Type", "application/json"),
                ],
                b,
            )
            .await;
        assert_eq!(
            resp.status, 403,
            "{method} {path} must be 403 for a read token (got {})",
            resp.status
        );
        assert_eq!(
            resp.json()["error"]["code"],
            "insufficient_scope",
            "{method} {path} returns insufficient_scope"
        );
    }
}

#[tokio::test]
async fn revoked_token_is_401() {
    let app = TestApp::new().await;
    let cookie = app.session_cookie().await;
    let secret = app.make_token("sender", &["send"], &cookie).await;
    let auth = format!("Bearer {secret}");

    // List tokens to find the id, then revoke it via the admin session.
    let list = app
        .get(
            "/api/v1/tokens",
            &[("Cookie", &cookie), ("Sec-Fetch-Site", "same-origin")],
        )
        .await;
    let id = list.json()[0]["id"].as_str().unwrap().to_string();
    let revoke = app
        .send(
            "DELETE",
            &format!("/api/v1/tokens/{id}"),
            &[("Cookie", &cookie), ("Sec-Fetch-Site", "same-origin")],
            None,
        )
        .await;
    assert_eq!(revoke.status, 200, "revoke succeeds");

    // The revoked token no longer authenticates.
    let after = app
        .send(
            "POST",
            "/api/v1/repeater/send",
            &[
                ("Authorization", &auth),
                ("Content-Type", "application/json"),
            ],
            Some(b"{\"host\":\"x\",\"port\":80,\"raw\":\"aGk=\"}"),
        )
        .await;
    assert_eq!(after.status, 401, "a revoked token is 401");
    assert_eq!(after.json()["error"]["code"], "invalid_token");
}

#[tokio::test]
async fn no_cors_headers_ever() {
    let app = TestApp::new().await;
    let cookie = app.session_cookie().await;
    let origin = "http://evil.test";

    let responses = vec![
        app.get("/api/v1/health", &[("Origin", origin)]).await,
        app.get("/api/v1/counts", &[("Origin", origin)]).await, // 401 (no cred)
        app.get("/api/v1/counts", &[("Cookie", &cookie), ("Origin", origin)])
            .await, // 403
        app.get("/api/v1/health", &[("Host", "evil.test")]).await, // 421
    ];
    for resp in &responses {
        for (name, _) in &resp.headers {
            assert!(
                !name.starts_with("access-control-"),
                "no Access-Control-* header may be emitted (saw {name})"
            );
        }
    }
}

#[tokio::test]
async fn session_idle_expiry_and_refresh() {
    // A short idle window, but wide enough that CI scheduling/IO overhead
    // between the sleeps and the request round-trip cannot push a request
    // past the window on its own (the sleeps carry a ~400ms margin below the
    // window; the expiry sleep a ~500ms margin above it).
    let app = TestApp::with(Opts {
        idle_timeout: Duration::from_millis(1000),
        ..Opts::default()
    })
    .await;
    let cookie = app.session_cookie().await;
    let h = [
        ("Cookie", cookie.as_str()),
        ("Sec-Fetch-Site", "same-origin"),
    ];

    // Two requests, each inside the window measured from the previous one.
    // Without refresh the second (1200ms after creation) would exceed 1000ms;
    // it succeeds, which proves each request refreshes last-seen.
    tokio::time::sleep(Duration::from_millis(600)).await;
    let r1 = app.get("/api/v1/counts", &h).await;
    assert_eq!(r1.status, 200, "within the window");
    tokio::time::sleep(Duration::from_millis(600)).await;
    let r2 = app.get("/api/v1/counts", &h).await;
    assert_eq!(r2.status, 200, "refreshed, so still within the window");

    // Now idle past the window: the session expires.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let r3 = app.get("/api/v1/counts", &h).await;
    assert_eq!(r3.status, 401, "an idle session expires");
    assert_eq!(r3.json()["error"]["code"], "session_invalid");
}

#[tokio::test]
async fn malformed_body_returns_envelope() {
    let app = TestApp::new().await;
    // A malformed JSON body is rejected by the extractor, but still as the
    // stable error envelope with a 4xx status.
    let resp = app
        .send(
            "POST",
            "/api/v1/session",
            &[("Content-Type", "application/json")],
            Some(b"this is not json"),
        )
        .await;
    assert!(
        (400..500).contains(&resp.status),
        "a malformed body is a 4xx (got {})",
        resp.status
    );
    assert_eq!(
        resp.json()["error"]["code"],
        "invalid_body",
        "the error envelope carries invalid_body"
    );
    assert!(
        resp.json()["error"]["message"].is_string(),
        "the envelope has a message"
    );
}
