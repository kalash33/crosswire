//! The `GET /events` WebSocket: event streaming, bearer first-frame auth,
//! `after` resync, and the `lagged` signal under a slow consumer.

mod common;

use std::time::Duration;

use common::{request_message, Opts, TestApp, Ws};
use crosswire_model::{Body, Exchange, Message};

const DEADLINE: Duration = Duration::from_secs(3);

fn simple() -> Exchange {
    Exchange::new(request_message(
        b"GET /e HTTP/1.1\r\nHost: a.test\r\n\r\n",
        Body::None,
    ))
}

/// Reads frames until one contains `needle` or the stream ends.
async fn wait_for_frame(ws: &mut Ws, needle: &str) -> bool {
    for _ in 0..50 {
        match ws.recv_text(DEADLINE).await {
            Some(frame) if frame.contains(needle) => return true,
            Some(_) => continue,
            None => return false,
        }
    }
    false
}

#[tokio::test]
async fn streams_received_and_committed() {
    let app = TestApp::new().await;
    let cookie = app.session_cookie().await;
    let mut ws = Ws::connect(
        app.addr,
        app.port,
        "/api/v1/events",
        &[("Cookie", &cookie), ("Sec-Fetch-Site", "same-origin")],
    )
    .await
    .expect("the upgrade succeeds");
    // Let the server subscribe before we submit.
    tokio::time::sleep(Duration::from_millis(150)).await;

    app.submit(simple());

    assert!(
        wait_for_frame(&mut ws, "\"received\"").await,
        "a received event arrives"
    );
    assert!(
        wait_for_frame(&mut ws, "\"committed\"").await,
        "a committed event arrives"
    );
}

#[tokio::test]
async fn cookie_without_same_origin_is_rejected() {
    let app = TestApp::new().await;
    let cookie = app.session_cookie().await;
    // A cookie upgrade without the same-origin proof is refused (403), no 101.
    let err = Ws::connect(app.addr, app.port, "/api/v1/events", &[("Cookie", &cookie)]).await;
    assert_eq!(err.err(), Some(403), "cookie WS upgrade needs same-origin");
}

#[tokio::test]
async fn bearer_first_frame_auth() {
    let app = TestApp::new().await;
    let cookie = app.session_cookie().await;
    let secret = app.make_token("ws", &["read"], &cookie).await;

    // No credential on the upgrade: the first frame carries the bearer token.
    let mut ws = Ws::connect(app.addr, app.port, "/api/v1/events", &[])
        .await
        .expect("the upgrade succeeds");
    ws.send_text(&format!("{{\"bearer\":\"{secret}\"}}")).await;
    // Let the server authenticate the frame and subscribe before we submit.
    tokio::time::sleep(Duration::from_millis(150)).await;

    app.submit(simple());
    assert!(
        wait_for_frame(&mut ws, "\"received\"").await,
        "events stream after first-frame auth"
    );
}

#[tokio::test]
async fn bad_first_frame_closes() {
    let app = TestApp::new().await;
    let mut ws = Ws::connect(app.addr, app.port, "/api/v1/events", &[])
        .await
        .expect("the upgrade succeeds");
    ws.send_text("{\"bearer\":\"cwt_not-a-real-token\"}").await;
    // An invalid first frame closes the socket with no events.
    assert!(
        ws.recv_text(DEADLINE).await.is_none(),
        "a bad first frame closes the stream"
    );
}

#[tokio::test]
async fn after_resync_sends_lagged_first() {
    let app = TestApp::new().await;
    let cookie = app.session_cookie().await;
    let mut ws = Ws::connect(
        app.addr,
        app.port,
        "/api/v1/events?after=5",
        &[("Cookie", &cookie), ("Sec-Fetch-Site", "same-origin")],
    )
    .await
    .expect("the upgrade succeeds");
    let first = ws.recv_text(DEADLINE).await.expect("a first frame");
    assert!(
        first.contains("\"lagged\""),
        "resync announces a gap: {first}"
    );
}

#[tokio::test]
async fn slow_consumer_gets_lagged() {
    // A tiny per-subscriber buffer lags as soon as the consumer stalls.
    let app = TestApp::with(Opts {
        event_capacity: 2,
        ..Opts::default()
    })
    .await;
    let cookie = app.session_cookie().await;
    let mut ws = Ws::connect(
        app.addr,
        app.port,
        "/api/v1/events",
        &[("Cookie", &cookie), ("Sec-Fetch-Site", "same-origin")],
    )
    .await
    .expect("the upgrade succeeds");

    // Flood far more events than the socket buffer and the broadcast capacity
    // can hold while the consumer is not reading.
    let msg = request_message(b"GET /f HTTP/1.1\r\nHost: a.test\r\n\r\n", Body::None);
    for _ in 0..5000 {
        app.store
            .sink()
            .submit(Exchange::new(clone_message(&msg)))
            .unwrap();
    }
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Now drain: a lagged frame must appear.
    let mut saw_lagged = false;
    for _ in 0..5000 {
        match ws.recv_text(DEADLINE).await {
            Some(frame) if frame.contains("\"lagged\"") => {
                saw_lagged = true;
                break;
            }
            Some(_) => continue,
            None => break,
        }
    }
    assert!(saw_lagged, "a slow consumer receives a lagged frame");
}

fn clone_message(m: &Message) -> Message {
    m.clone()
}

#[tokio::test]
async fn first_frame_bad_token_is_rate_limited() {
    let app = TestApp::new().await;

    // Ten first-frame bad-token attempts from the same peer are each closed.
    for _ in 0..10 {
        let mut ws = Ws::connect(app.addr, app.port, "/api/v1/events", &[])
            .await
            .expect("the upgrade succeeds");
        ws.send_text("{\"bearer\":\"cwt_not-a-real-token\"}").await;
        assert!(
            ws.recv_text(DEADLINE).await.is_none(),
            "a bad first-frame token closes the socket"
        );
    }

    // The 11th attempt is refused by the rate limit before the upgrade.
    let refused = Ws::connect(app.addr, app.port, "/api/v1/events", &[]).await;
    assert_eq!(
        refused.err(),
        Some(429),
        "the first-frame path enforces the auth-failure rate limit"
    );

    // The failures were audited.
    let log = std::fs::read_to_string(app.store.dir().join("audit.jsonl")).unwrap();
    let fails = log
        .lines()
        .filter(|l| l.contains("\"auth.fail\"") && l.contains("/api/v1/events"))
        .count();
    assert!(
        fails >= 10,
        "first-frame auth failures are audited (got {fails})"
    );
}
