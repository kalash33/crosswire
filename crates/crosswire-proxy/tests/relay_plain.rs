//! The relay over plain HTTP: absolute-form requests to the proxy,
//! rewritten to origin-form for a scripted TCP upstream.

mod common;

use std::net::IpAddr;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use common::relay::{self as s, read_n, Via};
use common::upstream::spawn_raw;
use common::{connect_raw, loopback_config, ProxyHarness, UpstreamWiring};
use crosswire_core::{ChangeOp, ChangeReason};
use crosswire_model::MarkerKind;
use crosswire_proxy::NoopHandler;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn get_round_trip_is_byte_exact() {
    s::get_round_trip_is_byte_exact(Via::Plain).await;
}

#[tokio::test]
async fn keep_alive_two_requests_same_upstream_connection() {
    s::keep_alive_two_requests_same_upstream_connection(Via::Plain).await;
}

#[tokio::test]
async fn connection_close_from_server_ends_relay() {
    s::connection_close_from_server_ends_relay(Via::Plain).await;
}

#[tokio::test]
async fn pipelined_requests_are_served_serially_in_order() {
    s::pipelined_requests_are_served_serially_in_order(Via::Plain).await;
}

#[tokio::test]
async fn request_body_content_length_forwarded_exactly() {
    s::request_body_content_length_forwarded_exactly(Via::Plain).await;
}

#[tokio::test]
async fn chunked_response_with_extensions_and_trailers_is_byte_exact() {
    s::chunked_response_with_extensions_and_trailers_is_byte_exact(Via::Plain).await;
}

#[tokio::test]
async fn head_and_304_responses_are_framed_without_body() {
    s::head_and_304_responses_are_framed_without_body(Via::Plain).await;
}

#[tokio::test]
async fn http10_until_close_response_ends_connection() {
    s::http10_until_close_response_ends_connection(Via::Plain).await;
}

#[tokio::test]
async fn expect_100_continue_interim_is_forwarded_and_recorded() {
    s::expect_100_continue_interim_is_forwarded_and_recorded(Via::Plain).await;
}

#[tokio::test]
async fn host_header_differs_from_authority_is_forwarded_unchanged() {
    s::host_header_differs_from_authority_is_forwarded_unchanged(Via::Plain).await;
}

#[tokio::test]
async fn handler_drop_closes_without_forwarding() {
    s::handler_drop_closes_without_forwarding(Via::Plain).await;
}

#[tokio::test]
async fn pipelining_depth_over_limit_closes() {
    s::pipelining_depth_over_limit_closes(Via::Plain).await;
}

#[tokio::test]
async fn absolute_form_without_host_gets_host_inserted() {
    let resp = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok".to_vec();
    let rig = s::rig(Via::Plain, vec![resp.clone()], None, &["u.test"]).await;
    let mut c = rig.open("u.test").await;
    let req = format!(
        "{}\r\nAccept: */*\r\n\r\n",
        rig.start_line("GET", "u.test", "/n")
    );
    c.write_all(req.as_bytes()).await.unwrap();
    assert_eq!(read_n(&mut c, resp.len()).await, resp);
    let host = format!("u.test:{}", rig.port);
    let expected = format!("GET /n HTTP/1.1\r\nHost: {host}\r\nAccept: */*\r\n\r\n");
    assert_eq!(
        rig.received_at_least(expected.len()).await,
        expected.as_bytes()
    );
    let ex = rig.exchanges(1).await;
    let diffs = ex[0].request().diffs();
    let insert = diffs
        .iter()
        .find(|d| d.op() == ChangeOp::Insert)
        .expect("an insert diff");
    assert_eq!(
        *insert.reason(),
        ChangeReason::Custom("host-from-target".into())
    );
    assert!(insert.span().is_empty());
    // Inserted right after the start line of the original head.
    let start_line_end = req.find("\r\n").unwrap() + 2;
    assert_eq!(insert.span().start() as usize, start_line_end);
    assert_eq!(&insert.after()[..], format!("Host: {host}\r\n").as_bytes());
}

#[tokio::test]
async fn extra_bytes_after_response_close_upstream_and_are_not_served() {
    let first = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok".to_vec();
    let junky = [first.clone(), b"JUNK!".to_vec()].concat();
    let second = b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\nsecond".to_vec();
    let rig = s::rig(Via::Plain, vec![junky, second.clone()], None, &["u.test"]).await;
    let mut c = rig.open("u.test").await;
    for (path, resp) in [("/1", &first), ("/2", &second)] {
        let req = format!(
            "{}\r\nHost: u.test\r\n\r\n",
            rig.start_line("GET", "u.test", path)
        );
        c.write_all(req.as_bytes()).await.unwrap();
        assert_eq!(&read_n(&mut c, resp.len()).await, resp, "{path}");
    }
    let ex = rig.exchanges(2).await;
    assert_eq!(
        rig.connections(),
        2,
        "the second request used a fresh upstream"
    );
    for e in &ex {
        let body = s::body_bytes(e.response().unwrap());
        assert!(!body.windows(5).any(|w| w == b"JUNK!"), "{body:?}");
    }
    let markers = rig.h.sink.markers.lock().unwrap().clone();
    assert!(
        markers.iter().any(|m| m.kind() == MarkerKind::Error
            && m.detail() == format!("extra bytes after response from u.test:{}", rig.port)),
        "{markers:?}"
    );
    // Nothing else is pending: the client has no stray bytes either.
    c.shutdown().await.unwrap();
    assert_eq!(s::read_close(&mut c).await, b"");
}

#[tokio::test]
async fn plain_origin_switch_uses_a_fresh_upstream() {
    // a.test also sends bytes after its response (as a server that puts a
    // body on a HEAD response does): they must never reach b.test's client.
    let a = spawn_raw(
        vec![b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\naJUNK".to_vec()],
        None,
    )
    .await;
    let b = spawn_raw(
        vec![b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\nb".to_vec()],
        None,
    )
    .await;
    let lo = IpAddr::from([127, 0, 0, 1]);
    let wiring = UpstreamWiring {
        extra_roots: Vec::new(),
        allow: vec![
            ("127.0.0.1".to_string(), a.port()),
            ("127.0.0.1".to_string(), b.port()),
        ],
        resolve: vec![("a.test".to_string(), lo), ("b.test".to_string(), lo)],
        ..UpstreamWiring::default()
    };
    let h = ProxyHarness::start_wired(loopback_config(), Arc::new(NoopHandler), wiring).await;
    let mut c = connect_raw(h.addr).await;
    let req_a = format!(
        "GET http://a.test:{}/x HTTP/1.1\r\nHost: a.test\r\n\r\n",
        a.port()
    );
    let req_b = format!(
        "GET http://b.test:{}/y HTTP/1.1\r\nHost: b.test\r\n\r\n",
        b.port()
    );
    for (req, body) in [(&req_a, b'a'), (&req_b, b'b')] {
        c.write_all(req.as_bytes()).await.unwrap();
        let want = [
            &b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\n"[..],
            &[body][..],
        ]
        .concat();
        let mut got = vec![0u8; want.len()];
        tokio::time::timeout(Duration::from_secs(5), c.read_exact(&mut got))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got, want);
    }
    s::wait_for("both requests upstream", || {
        !a.received.lock().unwrap().is_empty() && !b.received.lock().unwrap().is_empty()
    })
    .await;
    assert_eq!(
        a.received.lock().unwrap().as_slice(),
        b"GET /x HTTP/1.1\r\nHost: a.test\r\n\r\n"
    );
    assert_eq!(
        b.received.lock().unwrap().as_slice(),
        b"GET /y HTTP/1.1\r\nHost: b.test\r\n\r\n"
    );
    assert_eq!(a.connections.load(Ordering::SeqCst), 1);
    assert_eq!(b.connections.load(Ordering::SeqCst), 1);
    s::wait_for("two exchanges", || {
        h.sink.exchanges.lock().unwrap().len() == 2
    })
    .await;
    let ex = h.sink.exchanges.lock().unwrap().clone();
    let mut targets: Vec<_> = ex
        .iter()
        .map(|e| e.request().connection().target_addr().to_string())
        .collect();
    targets.sort();
    assert_eq!(targets, ["a.test", "b.test"]);
}
