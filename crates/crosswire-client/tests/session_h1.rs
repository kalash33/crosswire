//! HTTP/1 session: the re-present loop over real sockets.
#![allow(missing_docs)]
mod common;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use crosswire_client::{
    BodyPiece, ClientError, Connection, DestinationGate, HoldPoint, ResponseStream, RustlsBackend,
    Segment, Session, Target, Transport, WritePlan,
};
use crosswire_h1::{ParserConfig, ResponseContext};

async fn connect_plain(addr: SocketAddr) -> Connection {
    let mut g = DestinationGate::new(vec![]);
    g.allow("127.0.0.1", addr.port());
    let t = Transport::new(Arc::new(RustlsBackend::webpki_roots()), Arc::new(g));
    t.connect(&Target::plain("127.0.0.1", addr.port()))
        .await
        .unwrap()
}

fn req() -> Bytes {
    Bytes::from_static(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n")
}

async fn collect(r: &mut ResponseStream<'_>) -> Vec<BodyPiece> {
    let mut v = Vec::new();
    while let Some(p) = r.next_body().await.unwrap() {
        v.push(p);
    }
    v
}

#[tokio::test]
async fn content_length_response_roundtrip() {
    let addr =
        common::spawn_scripted(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello", &[]).await;
    let mut s = Session::new(connect_plain(addr).await, ParserConfig::default());
    let mut r = s
        .send(
            WritePlan::raw(Bytes::from_static(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n")),
            ResponseContext::for_method(b"GET"),
        )
        .await
        .unwrap();
    let head = r.head().await.unwrap();
    assert_eq!(head.head().status_code(head.bytes()), Some(200));
    let mut body = Vec::new();
    while let Some(p) = r.next_body().await.unwrap() {
        body.extend_from_slice(&p.data);
    }
    assert_eq!(body, b"hello");
    assert_eq!(
        r.raw_response(),
        b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello"
    );
    assert_eq!(r.request_bytes(), b"GET / HTTP/1.1\r\nHost: x\r\n\r\n");
    drop(r);
    assert!(s.is_reusable());
}

#[tokio::test]
async fn split_extension_across_reads() {
    let resp: &'static [u8] =
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5;x=yz\r\nhello\r\n0\r\nX-T: 1\r\n\r\n";
    let split_at = resp.len() - 22 + 3; // inside ";x=yz"
    let addr = common::spawn_scripted(resp, &[split_at, resp.len() - 6]).await;
    let mut s = Session::new(connect_plain(addr).await, ParserConfig::default());
    let mut r = s
        .send(WritePlan::raw(req()), ResponseContext::for_method(b"GET"))
        .await
        .unwrap();
    r.head().await.unwrap();
    let pieces: Vec<BodyPiece> = collect(&mut r).await;
    assert_eq!(pieces[0].ext.as_deref(), Some(&b"x=yz"[..]));
    let body: Vec<u8> = pieces.iter().flat_map(|p| p.data.to_vec()).collect();
    assert_eq!(body, b"hello");
    assert_eq!(
        pieces.last().unwrap().trailers.as_ref().unwrap()[0],
        (Bytes::from_static(b"X-T"), Bytes::from_static(b"1"))
    );
    assert_eq!(r.raw_response(), resp);
}

#[tokio::test]
async fn every_split_point_yields_identical_raw_response() {
    const RESP: &[u8] =
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5;x=yz\r\nhello\r\n3\r\nabc\r\n0\r\nX-T: 1\r\n\r\n";
    for cut in 1..RESP.len() {
        let addr = common::spawn_scripted(RESP, &[cut]).await;
        let mut s = Session::new(connect_plain(addr).await, ParserConfig::default());
        let mut r = s
            .send(WritePlan::raw(req()), ResponseContext::for_method(b"GET"))
            .await
            .unwrap();
        r.head().await.unwrap();
        let pieces = collect(&mut r).await;
        let body: Vec<u8> = pieces.iter().flat_map(|p| p.data.to_vec()).collect();
        assert_eq!(body, b"helloabc", "cut {cut}");
        assert_eq!(r.raw_response(), RESP, "cut {cut}");
        drop(r);
        assert!(s.is_reusable(), "cut {cut}");
    }
}

#[tokio::test]
async fn head_context_has_no_body() {
    let addr = common::spawn_scripted(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\n", &[]).await;
    let mut s = Session::new(connect_plain(addr).await, ParserConfig::default());
    let mut r = s
        .send(
            WritePlan::raw(Bytes::from_static(b"HEAD / HTTP/1.1\r\nHost: x\r\n\r\n")),
            ResponseContext::for_method(b"HEAD"),
        )
        .await
        .unwrap();
    r.head().await.unwrap();
    assert!(r.next_body().await.unwrap().is_none());
    assert_eq!(
        r.raw_response(),
        b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\n"
    );
    drop(r);
    assert!(s.is_reusable());
}

#[tokio::test]
async fn until_close_reads_to_eof_and_is_not_reusable() {
    let addr = common::spawn_scripted(b"HTTP/1.0 200 OK\r\n\r\nabcdef", &[22]).await;
    let mut s = Session::new(connect_plain(addr).await, ParserConfig::default());
    let mut r = s
        .send(WritePlan::raw(req()), ResponseContext::for_method(b"GET"))
        .await
        .unwrap();
    r.head().await.unwrap();
    let body: Vec<u8> = collect(&mut r)
        .await
        .iter()
        .flat_map(|p| p.data.to_vec())
        .collect();
    assert_eq!(body, b"abcdef");
    drop(r);
    assert!(!s.is_reusable());
}

#[tokio::test]
async fn keep_alive_reuse_two_exchanges() {
    let addr = common::spawn_scripted_keepalive(vec![
        b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\none",
        b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\ntwo",
    ])
    .await;
    let mut s = Session::new(connect_plain(addr).await, ParserConfig::default());
    for (i, want) in [&b"one"[..], &b"two"[..]].into_iter().enumerate() {
        let mut r = s
            .send(WritePlan::raw(req()), ResponseContext::for_method(b"GET"))
            .await
            .unwrap();
        r.head().await.unwrap();
        let body: Vec<u8> = collect(&mut r)
            .await
            .iter()
            .flat_map(|p| p.data.to_vec())
            .collect();
        assert_eq!(body, want);
        drop(r);
        assert!(s.is_reusable());
        assert_eq!(s.info().reused(), i == 1);
    }
}

#[tokio::test]
async fn connection_close_tokens_make_session_unreusable() {
    for resp in [
        &b"HTTP/1.1 200 OK\r\nconnection: Keep-Alive, CLOSE\r\nContent-Length: 0\r\n\r\n"[..],
        &b"HTTP/1.0 200 OK\r\nContent-Length: 0\r\n\r\n"[..],
    ] {
        let addr = common::spawn_scripted(Box::leak(resp.to_vec().into_boxed_slice()), &[]).await;
        let mut s = Session::new(connect_plain(addr).await, ParserConfig::default());
        let mut r = s
            .send(WritePlan::raw(req()), ResponseContext::for_method(b"GET"))
            .await
            .unwrap();
        r.head().await.unwrap();
        assert!(r.next_body().await.unwrap().is_none());
        drop(r);
        assert!(!s.is_reusable());
    }
    let addr = common::spawn_scripted(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n", &[]).await;
    let mut s = Session::new(connect_plain(addr).await, ParserConfig::default());
    let mut r = s
        .send(
            WritePlan::raw(Bytes::from_static(
                b"GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
            )),
            ResponseContext::for_method(b"GET"),
        )
        .await
        .unwrap();
    r.head().await.unwrap();
    assert!(r.next_body().await.unwrap().is_none());
    drop(r);
    assert!(!s.is_reusable());
}

#[tokio::test]
async fn bytes_after_the_response_stay_out_of_raw_and_fail_the_next_send() {
    let resp: &'static [u8] =
        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nokHTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n";
    let addr = common::spawn_scripted(resp, &[]).await;
    let mut s = Session::new(connect_plain(addr).await, ParserConfig::default());
    let mut r = s
        .send(WritePlan::raw(req()), ResponseContext::for_method(b"GET"))
        .await
        .unwrap();
    r.head().await.unwrap();
    collect(&mut r).await;
    assert_eq!(
        r.raw_response(),
        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok"
    );
    drop(r);
    assert!(!s.is_reusable());
    let e = s
        .send(WritePlan::raw(req()), ResponseContext::for_method(b"GET"))
        .await
        .err()
        .unwrap();
    assert!(
        matches!(&e, ClientError::Protocol(m) if m == "unexpected bytes after response"),
        "{e:?}"
    );
}

#[tokio::test]
async fn truncated_body_is_a_protocol_error_and_not_reusable() {
    let addr =
        common::spawn_scripted(b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\n\r\nabc", &[]).await;
    let mut s = Session::new(connect_plain(addr).await, ParserConfig::default());
    let mut r = s
        .send(WritePlan::raw(req()), ResponseContext::for_method(b"GET"))
        .await
        .unwrap();
    r.head().await.unwrap();
    assert!(r.next_body().await.unwrap().is_some());
    let e = r.next_body().await.err().unwrap();
    assert!(matches!(e, ClientError::Protocol(_)), "{e:?}");
    drop(r);
    assert!(!s.is_reusable());
}

#[tokio::test]
async fn hold_in_plan_is_rejected() {
    let addr = common::spawn_scripted(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n", &[]).await;
    let mut s = Session::new(connect_plain(addr).await, ParserConfig::default());
    let plan = WritePlan::from_segments(vec![
        Segment::new(req()),
        Segment::new(Bytes::from_static(b"x")).with_hold(HoldPoint::Manual),
    ]);
    let e = s
        .send(plan, ResponseContext::for_method(b"GET"))
        .await
        .err()
        .unwrap();
    assert!(
        matches!(&e, ClientError::Protocol(m) if m == "holds are not supported yet"),
        "{e:?}"
    );
}

#[tokio::test]
async fn multi_segment_plan_is_written_in_order_and_recorded() {
    let addr = common::spawn_scripted(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n", &[]).await;
    let mut s = Session::new(connect_plain(addr).await, ParserConfig::default());
    let plan = WritePlan::from_segments(vec![
        Segment::new(Bytes::from_static(b"GET / HTTP/1.1\r\n")),
        Segment::new(Bytes::from_static(b"Host: x\r\n\r\n")),
    ]);
    let mut r = s
        .send(plan, ResponseContext::for_method(b"GET"))
        .await
        .unwrap();
    r.head().await.unwrap();
    assert_eq!(r.request_bytes(), b"GET / HTTP/1.1\r\nHost: x\r\n\r\n");
}

#[tokio::test]
async fn idle_timeout() {
    let addr = common::spawn_silent().await;
    let mut s = Session::new(connect_plain(addr).await, ParserConfig::default())
        .with_read_timeout(Duration::from_millis(200));
    let mut r = s
        .send(WritePlan::raw(req()), ResponseContext::for_method(b"GET"))
        .await
        .unwrap();
    let e = r.head().await.err().unwrap();
    assert!(matches!(e, ClientError::Timeout), "{e:?}");
    drop(r);
    assert!(!s.is_reusable());
}
