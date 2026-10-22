//! Fetch a URL through `crosswire-client` and record it into a project.
mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::{Bytes, BytesMut};
use crosswire_client::{
    DestinationGate, RustlsBackend, Session, Target, Transport, VerifyMode, WritePlan,
};
use crosswire_h1::{write_exact, ParserConfig, ResponseContext};
use crosswire_model::{Body, ChainResult, Event, Head};
use crosswire_recorder_tests::{record_exchange, RecordError};
use crosswire_store::{BodySide, EventStream, Project, Store, StoreConfig};
use ulid::Ulid;

fn open(dir: &std::path::Path) -> Store {
    let cfg = StoreConfig {
        batch_interval: Duration::from_millis(10),
        ..StoreConfig::default()
    };
    Store::open(Project::open_or_create(dir).unwrap(), cfg).unwrap()
}

async fn wait_committed(ev: &mut EventStream, id: Ulid) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match tokio::time::timeout(left, ev.recv()).await {
            Ok(Some(Event::Committed { ids, .. })) if ids.contains(&id) => return,
            Ok(Some(_)) => {}
            other => panic!("no Committed for {id}: {other:?}"),
        }
    }
}

fn gate(port: u16) -> Arc<DestinationGate> {
    let mut g = DestinationGate::new(vec![]);
    g.allow("127.0.0.1", port);
    Arc::new(g)
}

/// A chunked response (8000-byte chunks) carrying `payload`, and the raw
/// framed body bytes a recorder must keep.
fn chunked_response(payload: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let head = b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
    let mut body = Vec::new();
    for c in payload.chunks(8000) {
        body.extend_from_slice(format!("{:x}\r\n", c.len()).as_bytes());
        body.extend_from_slice(c);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(b"0\r\n\r\n");
    let mut all = head.to_vec();
    all.extend_from_slice(&body);
    (all, body)
}

fn payload(n: usize) -> Vec<u8> {
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

const REQ: &[u8] = b"GET /data HTTP/1.1\r\nHost: example.test\r\nConnection: close\r\n\r\n";

async fn fetch_tls_and_record(dir: &std::path::Path) -> (Ulid, Vec<u8>) {
    let data = payload(40 * 1024);
    let (response, raw_body) = chunked_response(&data);
    let (ca, addr) = common::spawn_tls_scripted(response).await;
    let t = Transport::new(
        Arc::new(RustlsBackend::with_extra_roots(vec![ca.cert_der()])),
        gate(addr.port()),
    );
    let conn = t
        .connect(&Target::tls(
            "127.0.0.1",
            addr.port(),
            "example.test",
            VerifyMode::WebPki,
        ))
        .await
        .unwrap();
    let mut session = Session::new(conn, ParserConfig::default());
    let store = open(dir);
    let mut ev = store.subscribe();
    let sink = store.sink();
    let id = record_exchange(
        &mut session,
        WritePlan::raw(Bytes::from_static(REQ)),
        ResponseContext::for_method(b"GET"),
        Arc::clone(&sink),
    )
    .await
    .unwrap();
    wait_committed(&mut ev, id).await;
    store.close().unwrap();
    (id, raw_body)
}

fn check_recorded(store: &Store, id: Ulid, raw_body: &[u8]) {
    let ex = store.get(id).unwrap().expect("exchange exists");
    let req = ex.request();
    let Head::Http1Raw { bytes, .. } = req.head() else {
        panic!("request head is not raw");
    };
    // The recorded request head is exactly the bytes written.
    let mut out = BytesMut::new();
    write_exact_head(&mut out, req);
    assert_eq!(&out[..], &bytes[..]);
    assert_eq!(&bytes[..], REQ);
    let conn = req.connection();
    assert_eq!(*conn.upstream_chain_result(), ChainResult::Valid);
    assert!(conn.tls_version().is_some());

    let resp = ex.response().expect("response recorded");
    match resp.body() {
        Body::Blob { len, .. } => assert_eq!(*len, raw_body.len() as u64),
        other => panic!("response body not in a pack: {other:?}"),
    }
    assert_eq!(
        store.get_body(id, BodySide::Resp).unwrap().unwrap(),
        raw_body
    );
    assert_eq!(
        *resp.connection().upstream_chain_result(),
        ChainResult::Valid
    );
    assert_eq!(store.counts().unwrap().committed, 1);
    // The Host header, not the connection IP, is the recorded host.
    let by_host = crosswire_store::ExchangeFilter {
        host: Some("example.test".into()),
        ..crosswire_store::ExchangeFilter::default()
    };
    let rows = store.list(&by_host, None, 10).unwrap().items;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, id);
}

/// Re-serialises a recorded raw request head with `write_exact`, to prove the
/// recorded spans reproduce the written bytes.
fn write_exact_head(out: &mut BytesMut, msg: &crosswire_model::Message) {
    let Head::Http1Raw { bytes, .. } = msg.head() else {
        panic!("not raw");
    };
    let mut p = crosswire_h1::Parser::request(&ParserConfig::default());
    let step = p.feed(bytes).unwrap();
    let crosswire_h1::Progress::Head(raw) = step.event else {
        panic!("no head");
    };
    write_exact(&raw, out);
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_over_tls_and_record() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("p");
    let (id, raw_body) = fetch_tls_and_record(&dir).await;
    let store = open(&dir);
    check_recorded(&store, id, &raw_body);
    store.close().unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_plain_with_until_close() {
    let addr = common::spawn_plain_scripted(
        b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\nuntil the socket closes".to_vec(),
    )
    .await;
    let t = Transport::new(Arc::new(RustlsBackend::webpki_roots()), gate(addr.port()));
    let conn = t
        .connect(&Target::plain("127.0.0.1", addr.port()))
        .await
        .unwrap();
    let mut session = Session::new(conn, ParserConfig::default());
    let tmp = tempfile::tempdir().unwrap();
    let store = open(&tmp.path().join("p"));
    let mut ev = store.subscribe();
    let sink = store.sink();
    let id = record_exchange(
        &mut session,
        WritePlan::raw(Bytes::from_static(
            b"GET / HTTP/1.1\r\nHost: plain.test\r\n\r\n",
        )),
        ResponseContext::for_method(b"GET"),
        Arc::clone(&sink),
    )
    .await
    .unwrap();
    wait_committed(&mut ev, id).await;
    let ex = store.get(id).unwrap().unwrap();
    let resp = ex.response().unwrap();
    assert_eq!(
        store
            .list(&crosswire_store::ExchangeFilter::default(), None, 10)
            .unwrap()
            .items[0]
            .status,
        Some(200)
    );
    assert_eq!(resp.framing(), &crosswire_model::FramingRecord::UntilClose);
    assert_eq!(
        store.get_body(id, BodySide::Resp).unwrap().unwrap(),
        b"until the socket closes"
    );
    assert!(!session.is_reusable());
    store.close().unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn recorded_exchange_survives_reopen() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("p");
    let (id, raw_body) = fetch_tls_and_record(&dir).await;
    // `fetch_tls_and_record` closed the store cleanly; this is a reopen.
    let store = open(&dir);
    assert!(store.open_report().unclean_gap.is_none());
    check_recorded(&store, id, &raw_body);
    store.close().unwrap();
    let store = open(&dir);
    check_recorded(&store, id, &raw_body);
    store.close().unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn response_over_raw_cap_records_nothing() {
    let (response, _) = chunked_response(&payload(40 * 1024));
    let addr = common::spawn_plain_scripted(response).await;
    let t = Transport::new(Arc::new(RustlsBackend::webpki_roots()), gate(addr.port()));
    let conn = t
        .connect(&Target::plain("127.0.0.1", addr.port()))
        .await
        .unwrap();
    let mut session = Session::new(conn, ParserConfig::default()).with_raw_cap(1024);
    let tmp = tempfile::tempdir().unwrap();
    let store = open(&tmp.path().join("p"));
    let sink = store.sink();
    let err = record_exchange(
        &mut session,
        WritePlan::raw(Bytes::from_static(REQ)),
        ResponseContext::for_method(b"GET"),
        Arc::clone(&sink),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, RecordError::Truncated(_)), "{err:?}");
    // Nothing was submitted, so nothing can commit.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(store.counts().unwrap().exchanges, 0);
    store.close().unwrap();
}

/// Fetches a plain response and records it, returning the committed id.
async fn record_plain(store: &Store) -> Ulid {
    let addr =
        common::spawn_plain_scripted(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok".to_vec())
            .await;
    let t = Transport::new(Arc::new(RustlsBackend::webpki_roots()), gate(addr.port()));
    let conn = t
        .connect(&Target::plain("127.0.0.1", addr.port()))
        .await
        .unwrap();
    let mut session = Session::new(conn, ParserConfig::default());
    let mut ev = store.subscribe();
    let id = record_exchange(
        &mut session,
        WritePlan::raw(Bytes::from_static(
            b"GET / HTTP/1.1\r\nHost: plain.test\r\nConnection: close\r\n\r\n",
        )),
        ResponseContext::for_method(b"GET"),
        store.sink(),
    )
    .await
    .unwrap();
    wait_committed(&mut ev, id).await;
    id
}

#[tokio::test(flavor = "multi_thread")]
async fn records_inside_a_local_set() {
    let tmp = tempfile::tempdir().unwrap();
    let store = open(&tmp.path().join("p"));
    let local = tokio::task::LocalSet::new();
    let id = local.run_until(record_plain(&store)).await;
    assert!(store.get(id).unwrap().is_some());
    store.close().unwrap();
}

#[tokio::test]
async fn records_on_a_current_thread_runtime() {
    let tmp = tempfile::tempdir().unwrap();
    let store = open(&tmp.path().join("p"));
    let id = record_plain(&store).await;
    assert!(store.get(id).unwrap().is_some());
    store.close().unwrap();
}
