//! Recording into a real store: bodies, the capture cap, the in-flight
//! budget, diagnostics, interim heads, a slow client and a full sink.

mod common;

use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::relay::wait_for;
use common::upstream::spawn_raw;
use common::{connect_raw, loopback_config, read_until, ProxyHarness, UpstreamWiring};
use crosswire_core::DiagnosticCode;
use crosswire_model::{Body, Capture, Exchange, ExchangeSink, Marker, SinkError};
use crosswire_proxy::ProxyLimits;
use crosswire_store::{BodySide, EventStream, ExchangeFilter, Project, Store, StoreConfig};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use ulid::Ulid;

const KIB: usize = 1024;
const MIB: usize = 1024 * 1024;
const WAIT: Duration = Duration::from_secs(20);

/// A store in a fresh temporary project.
struct Rec {
    _dir: TempDir,
    store: Store,
    events: EventStream,
}

fn open_store() -> Rec {
    let dir = tempfile::tempdir().unwrap();
    let cfg = StoreConfig {
        batch_interval: Duration::from_millis(10),
        ..StoreConfig::default()
    };
    let store = Store::open(Project::open_or_create(dir.path()).unwrap(), cfg).unwrap();
    let events = store.subscribe();
    Rec {
        _dir: dir,
        store,
        events,
    }
}

impl Rec {
    /// Waits until `n` exchanges to `u.test:*` are committed; returns their
    /// ids, oldest first.
    async fn committed(&mut self, n: usize) -> Vec<Ulid> {
        let filter = ExchangeFilter {
            host_contains: Some("u.test".into()),
            ..ExchangeFilter::default()
        };
        let deadline = Instant::now() + WAIT;
        loop {
            let page = self.store.list(&filter, None, 10).unwrap();
            if page.items.len() >= n {
                assert_eq!(page.items.len(), n, "more exchanges than expected");
                let mut ids: Vec<Ulid> = page.items.iter().map(|r| r.id).collect();
                ids.reverse();
                return ids;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            match tokio::time::timeout(left, self.events.recv()).await {
                Ok(Some(_)) => {}
                Ok(None) => panic!("event stream ended"),
                Err(_) => panic!("{n} exchanges not committed in time"),
            }
        }
    }

    fn get(&self, id: Ulid) -> Exchange {
        self.store.get(id).unwrap().expect("exchange exists")
    }

    /// The exchange JSON exactly as stored.
    fn stored_json(&self, id: Ulid) -> serde_json::Value {
        let conn = self.store.reader().unwrap();
        let json: String = conn
            .query_row(
                "SELECT json FROM exchanges WHERE ulid = ?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        serde_json::from_str(&json).unwrap()
    }

    fn close(self) {
        self.store.close().unwrap();
    }
}

/// `(truncated_at, capture_dropped, relay)` of a body, inline or blob.
fn capture_meta(b: &Body) -> (Option<u64>, bool, Option<Capture>) {
    match b {
        Body::Inline {
            truncated_at,
            capture_dropped,
            relay,
            ..
        }
        | Body::Blob {
            truncated_at,
            capture_dropped,
            relay,
            ..
        } => (*truncated_at, *capture_dropped, *relay),
        other => panic!("unexpected body {other:?}"),
    }
}

fn wiring(port: u16) -> UpstreamWiring {
    UpstreamWiring {
        extra_roots: Vec::new(),
        allow: vec![("127.0.0.1".to_string(), port)],
        resolve: vec![("u.test".to_string(), IpAddr::from([127, 0, 0, 1]))],
    }
}

async fn proxy(sink: Arc<dyn ExchangeSink>, limits: ProxyLimits, port: u16) -> ProxyHarness {
    let mut cfg = loopback_config();
    cfg.limits = limits;
    ProxyHarness::start_with_sink(sink, cfg, wiring(port)).await
}

/// Reads exactly `n` bytes.
async fn read_exact_n(s: &mut TcpStream, n: usize) -> Vec<u8> {
    let mut out = vec![0u8; n];
    tokio::time::timeout(WAIT, s.read_exact(&mut out))
        .await
        .expect("response not read in time")
        .unwrap();
    out
}

fn sha(b: &[u8]) -> [u8; 32] {
    Sha256::digest(b).into()
}

#[tokio::test]
async fn exchange_is_queryable_with_both_bodies() {
    let resp = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello".to_vec();
    let up = spawn_raw(vec![resp.clone()], None).await;
    let mut rec = open_store();
    let h = proxy(rec.store.sink(), ProxyLimits::default(), up.port()).await;

    let mut c = connect_raw(h.addr).await;
    let req = format!(
        "POST http://u.test:{}/x HTTP/1.1\r\nHost: u.test:{}\r\nContent-Length: 4\r\n\r\nping",
        up.port(),
        up.port()
    );
    c.write_all(req.as_bytes()).await.unwrap();
    assert_eq!(read_exact_n(&mut c, resp.len()).await, resp);

    let ids = rec.committed(1).await;
    // `req_host` is the Host header as recorded, port included.
    let by_host = ExchangeFilter {
        host: Some(format!("u.test:{}", up.port())),
        ..ExchangeFilter::default()
    };
    let rows = rec.store.list(&by_host, None, 10).unwrap().items;
    assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), ids);
    let ex = rec.get(ids[0]);
    assert!(ex.response().is_some());
    assert_eq!(
        rec.store.get_body(ids[0], BodySide::Req).unwrap().unwrap(),
        b"ping"
    );
    assert_eq!(
        rec.store.get_body(ids[0], BodySide::Resp).unwrap().unwrap(),
        b"hello"
    );
    // Whole bodies carry no relay record.
    assert_eq!(capture_meta(ex.request().body()), (None, false, None));
    assert_eq!(
        capture_meta(ex.response().unwrap().body()),
        (None, false, None)
    );
    drop(c);
    h.shutdown.shutdown().await;
    rec.close();
}

#[tokio::test]
async fn truncated_response_is_recorded_with_relay_capture() {
    let body: Vec<u8> = (0..70 * KIB).map(|i| (i % 251) as u8).collect();
    let mut resp =
        format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
    let head_len = resp.len();
    resp.extend_from_slice(&body);
    let up = spawn_raw(vec![resp.clone()], None).await;
    let mut rec = open_store();
    let limits = ProxyLimits::builder().capture_cap(1024).build();
    let h = proxy(rec.store.sink(), limits, up.port()).await;

    let mut c = connect_raw(h.addr).await;
    let req = format!("GET http://u.test:{}/big HTTP/1.1\r\n\r\n", up.port());
    c.write_all(req.as_bytes()).await.unwrap();
    // The client gets all of it, whatever the capture kept.
    let got = read_exact_n(&mut c, resp.len()).await;
    assert_eq!(&got[head_len..], &body[..]);

    let ids = rec.committed(1).await;
    let stored = rec.store.get_body(ids[0], BodySide::Resp).unwrap().unwrap();
    assert_eq!(stored, &body[..1024]);
    let json = rec.stored_json(ids[0]);
    assert_eq!(
        json.pointer("/response/body/truncated_at"),
        Some(&1024.into())
    );
    assert_eq!(
        json.pointer("/response/body/relay/len"),
        Some(&71680.into())
    );
    let ex = rec.get(ids[0]);
    let (truncated_at, dropped, relay) = capture_meta(ex.response().unwrap().body());
    assert_eq!(truncated_at, Some(1024));
    assert!(!dropped);
    let relay = relay.expect("relay capture recorded");
    assert_eq!(relay.len, 71680);
    assert_eq!(relay.sha256, sha(&body));
    drop(c);
    h.shutdown.shutdown().await;
    rec.close();
}

#[tokio::test]
async fn budget_dropped_body_round_trips_through_store() {
    let body = vec![b'z'; 4 * KIB];
    let mut resp =
        format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
    resp.extend_from_slice(&body);
    let up = spawn_raw(vec![resp.clone()], None).await;
    let mut rec = open_store();
    // Smaller than one 64 KiB reservation step: every capture is refused.
    let limits = ProxyLimits::builder().body_budget(1024).build();
    let h = proxy(rec.store.sink(), limits, up.port()).await;

    let mut c = connect_raw(h.addr).await;
    let req = format!("GET http://u.test:{}/ HTTP/1.1\r\n\r\n", up.port());
    c.write_all(req.as_bytes()).await.unwrap();
    assert_eq!(read_exact_n(&mut c, resp.len()).await, resp);

    let ids = rec.committed(1).await;
    let ex = rec.get(ids[0]);
    let (truncated_at, dropped, relay) = capture_meta(ex.response().unwrap().body());
    assert!(dropped);
    assert_eq!(truncated_at, None);
    let relay = relay.expect("relay capture recorded");
    assert_eq!(relay.len, 4096);
    assert_eq!(relay.sha256, sha(&body));
    // The store keeps a zero-length body for the dropped capture.
    assert_eq!(
        rec.store.get_body(ids[0], BodySide::Resp).unwrap(),
        Some(Vec::new())
    );
    // A request without a body stays `Body::None`.
    assert_eq!(ex.request().body(), &Body::None);
    assert_eq!(rec.store.get_body(ids[0], BodySide::Req).unwrap(), None);
    drop(c);
    h.shutdown.shutdown().await;
    rec.close();
}

#[tokio::test]
async fn body_diagnostics_are_recorded() {
    let resp =
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5;foo=bar\r\nhello\r\n0\r\n\r\n"
            .to_vec();
    let up = spawn_raw(vec![resp.clone()], None).await;
    let mut rec = open_store();
    let h = proxy(rec.store.sink(), ProxyLimits::default(), up.port()).await;

    let mut c = connect_raw(h.addr).await;
    let req = format!("GET http://u.test:{}/c HTTP/1.1\r\n\r\n", up.port());
    c.write_all(req.as_bytes()).await.unwrap();
    assert_eq!(read_exact_n(&mut c, resp.len()).await, resp);

    let ids = rec.committed(1).await;
    let ex = rec.get(ids[0]);
    let response = ex.response().unwrap();
    assert!(
        response
            .diagnostics()
            .iter()
            .any(|d| d.code() == DiagnosticCode::ChunkExtensionPresent),
        "diagnostics: {:?}",
        response.diagnostics()
    );
    // Raw framed bytes are stored.
    assert_eq!(
        rec.store.get_body(ids[0], BodySide::Resp).unwrap().unwrap(),
        b"5;foo=bar\r\nhello\r\n0\r\n\r\n"
    );
    drop(c);
    h.shutdown.shutdown().await;
    rec.close();
}

#[tokio::test]
async fn interim_heads_round_trip_through_store() {
    let interim = b"HTTP/1.1 100 Continue\r\n\r\n";
    let mut resp = interim.to_vec();
    resp.extend_from_slice(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok");
    let up = spawn_raw(vec![resp.clone()], None).await;
    let mut rec = open_store();
    let h = proxy(rec.store.sink(), ProxyLimits::default(), up.port()).await;

    let mut c = connect_raw(h.addr).await;
    let req = format!(
        "POST http://u.test:{}/u HTTP/1.1\r\nExpect: 100-continue\r\nContent-Length: 3\r\n\r\nabc",
        up.port()
    );
    c.write_all(req.as_bytes()).await.unwrap();
    assert_eq!(read_exact_n(&mut c, resp.len()).await, resp);

    let ids = rec.committed(1).await;
    let ex = rec.get(ids[0]);
    let interims: Vec<&[u8]> = ex
        .response()
        .unwrap()
        .interim()
        .iter()
        .map(|b| &b[..])
        .collect();
    assert_eq!(interims, vec![&interim[..]]);
    drop(c);
    h.shutdown.shutdown().await;
    rec.close();
}

/// An upstream that answers one request with `len` body bytes, written in
/// 64 KiB pieces as fast as the proxy reads them, counting what was
/// written.
async fn spawn_firehose(len: usize) -> (u16, Arc<AtomicU64>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let written = Arc::new(AtomicU64::new(0));
    let w = Arc::clone(&written);
    tokio::spawn(async move {
        let (mut s, _) = listener.accept().await.unwrap();
        let _ = read_until(&mut s, b"\r\n\r\n").await;
        let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {len}\r\n\r\n");
        s.write_all(head.as_bytes()).await.unwrap();
        let piece = vec![b'f'; 64 * KIB];
        let mut left = len;
        while left > 0 {
            let n = left.min(piece.len());
            if s.write_all(&piece[..n]).await.is_err() {
                return;
            }
            w.fetch_add(n as u64, Ordering::SeqCst);
            left -= n;
        }
        let _ = s.flush().await;
        // Hold the connection until the proxy closes it.
        let mut buf = [0u8; 64];
        let _ = s.read(&mut buf).await;
    });
    (port, written)
}

#[tokio::test]
async fn slow_client_does_not_buffer_upstream() {
    const TOTAL: usize = 50 * MIB;
    const CAP: u64 = MIB as u64;
    let (port, written) = spawn_firehose(TOTAL).await;
    let mut rec = open_store();
    let limits = ProxyLimits::builder().capture_cap(CAP).build();
    let h = proxy(rec.store.sink(), limits, port).await;

    let mut c = connect_raw(h.addr).await;
    let req = format!("GET http://u.test:{port}/stream HTTP/1.1\r\n\r\n");
    c.write_all(req.as_bytes()).await.unwrap();
    // 1 KiB every 20 ms for 2 s.
    let mut got = 0usize;
    let mut buf = vec![0u8; KIB];
    let end = Instant::now() + Duration::from_secs(2);
    while Instant::now() < end {
        let n = c.read(&mut buf).await.unwrap();
        assert!(n > 0, "proxy closed early");
        got += n;
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // What the upstream got rid of is bounded by kernel socket buffers on
    // both sides plus the relay's one 64 KiB read buffer (asserted by the
    // pump's unit test), not by the 50 MiB on offer.
    let w = written.load(Ordering::SeqCst);
    eprintln!("slow phase: client read {got} bytes, upstream wrote {w}");
    assert!(
        w < 8 * MIB as u64,
        "upstream wrote {w} bytes to a slow client"
    );

    // Drain the rest at full speed.
    let mut big = vec![0u8; 256 * KIB];
    let head_len = format!("HTTP/1.1 200 OK\r\nContent-Length: {TOTAL}\r\n\r\n").len();
    tokio::time::timeout(Duration::from_secs(60), async {
        while got < head_len + TOTAL {
            let n = c.read(&mut big).await.unwrap();
            assert!(n > 0, "proxy closed before the body ended");
            got += n;
        }
    })
    .await
    .expect("body not drained in time");
    assert_eq!(got, head_len + TOTAL);

    let ids = rec.committed(1).await;
    let ex = rec.get(ids[0]);
    let (truncated_at, dropped, relay) = capture_meta(ex.response().unwrap().body());
    assert_eq!(truncated_at, Some(CAP));
    assert!(!dropped);
    assert_eq!(relay.expect("relay capture").len, TOTAL as u64);
    assert_eq!(
        rec.store
            .get_body(ids[0], BodySide::Resp)
            .unwrap()
            .unwrap()
            .len(),
        MIB
    );
    drop(c);
    h.shutdown.shutdown().await;
    rec.close();
}

/// A sink that is always full.
#[derive(Default)]
struct FullSink {
    calls: AtomicUsize,
}

impl ExchangeSink for FullSink {
    fn submit(&self, _: Exchange) -> Result<(), SinkError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(SinkError::Full)
    }

    fn submit_marker(&self, _: Marker) -> Result<(), SinkError> {
        Err(SinkError::Full)
    }
}

#[tokio::test]
async fn sink_full_never_stalls_relay() {
    let resp = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok".to_vec();
    let up = spawn_raw(vec![resp.clone(), resp.clone(), resp.clone()], None).await;
    let sink = Arc::new(FullSink::default());
    let h = proxy(
        Arc::clone(&sink) as Arc<dyn ExchangeSink>,
        ProxyLimits::default(),
        up.port(),
    )
    .await;

    let mut c = connect_raw(h.addr).await;
    for i in 0..3 {
        let req = format!("GET http://u.test:{}/{i} HTTP/1.1\r\n\r\n", up.port());
        c.write_all(req.as_bytes()).await.unwrap();
        assert_eq!(read_exact_n(&mut c, resp.len()).await, resp);
    }
    wait_for("three refusals", || {
        h.counters.sink_full.load(Ordering::Relaxed) == 3
    })
    .await;
    assert_eq!(sink.calls.load(Ordering::SeqCst), 3);
    assert_eq!(h.counters.exchanges.load(Ordering::Relaxed), 3);
    drop(c);
    h.shutdown.shutdown().await;
}
