//! Relay scenarios shared by `relay_plain.rs` (absolute-form over plain
//! HTTP) and `relay_tls.rs` (CONNECT + MITM): each scenario runs against a
//! scripted upstream reached through the chosen path.

use std::net::IpAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use crosswire_core::{ChangeOp, ChangeReason, DiagnosticCode};
use crosswire_h1::{Parser, ParserConfig, Progress, RawMessage, ResponseContext};
use crosswire_model::{Body, ConnectionInfo, Exchange, Head, MarkerKind, Message};
use crosswire_proxy::{Decision, Handler, NoopHandler, ProxyConfig, ProxyLimits};
use rustls::{ClientConfig, RootCertStore};
use rustls_pki_types::{CertificateDer, ServerName};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio_rustls::TlsConnector;

use super::upstream::{spawn_raw, spawn_tls_scripted};
use super::{connect_raw, loopback_config, read_until, ProxyHarness, UpstreamWiring};

const WAIT: Duration = Duration::from_secs(5);

/// A client byte stream, plain or TLS.
pub trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

/// How the client reaches the upstream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Via {
    /// Absolute-form requests to the proxy over plain HTTP.
    Plain,
    /// `CONNECT`, TLS with a Crosswire leaf, origin-form requests inside.
    Tls,
}

/// A proxy wired to one scripted upstream.
pub struct Rig {
    pub via: Via,
    pub h: ProxyHarness,
    pub port: u16,
    pub received: Arc<Mutex<Vec<u8>>>,
    pub connections: Arc<AtomicUsize>,
}

/// Starts a scripted upstream for `via` and a proxy that reaches it under
/// each of `names` (and as 127.0.0.1).
pub async fn rig_with(
    via: Via,
    script: Vec<Vec<u8>>,
    fragment: Option<usize>,
    names: &[&str],
    cfg: ProxyConfig,
    handler: Arc<dyn Handler>,
) -> Rig {
    let (port, received, connections, wiring) = match via {
        Via::Plain => {
            let up = spawn_raw(script, fragment).await;
            let wiring = UpstreamWiring {
                extra_roots: Vec::new(),
                allow: vec![("127.0.0.1".to_string(), up.port())],
                resolve: names
                    .iter()
                    .map(|n| ((*n).to_string(), IpAddr::from([127, 0, 0, 1])))
                    .collect(),
            };
            (up.port(), up.received, up.connections, wiring)
        }
        Via::Tls => {
            let up = spawn_tls_scripted(script, fragment).await;
            let wiring = UpstreamWiring::loopback(up.ca_der(), up.port(), names);
            (up.port(), up.received, up.connections, wiring)
        }
    };
    let h = ProxyHarness::start_wired(cfg, handler, wiring).await;
    Rig {
        via,
        h,
        port,
        received,
        connections,
    }
}

/// [`rig_with`] with the default config and handler.
pub async fn rig(via: Via, script: Vec<Vec<u8>>, fragment: Option<usize>, names: &[&str]) -> Rig {
    rig_with(
        via,
        script,
        fragment,
        names,
        loopback_config(),
        Arc::new(NoopHandler),
    )
    .await
}

impl Rig {
    /// Opens a client connection that will talk to `host`.
    pub async fn open(&self, host: &str) -> Box<dyn Io> {
        match self.via {
            Via::Plain => Box::new(connect_raw(self.h.addr).await),
            Via::Tls => {
                let mut s = connect_raw(self.h.addr).await;
                let authority = format!("{host}:{}", self.port);
                let req = format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n\r\n");
                s.write_all(req.as_bytes()).await.unwrap();
                let got = read_until(&mut s, b"\r\n\r\n").await;
                assert_eq!(got, b"HTTP/1.1 200 Connection established\r\n\r\n");
                let mut store = RootCertStore::empty();
                store
                    .add(CertificateDer::from(self.h.ca.cert_der()))
                    .unwrap();
                let provider = Arc::new(rustls::crypto::ring::default_provider());
                let mut cfg = ClientConfig::builder_with_provider(provider)
                    .with_safe_default_protocol_versions()
                    .unwrap()
                    .with_root_certificates(store)
                    .with_no_client_auth();
                cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
                let name = ServerName::try_from(host.to_string()).unwrap();
                let tls =
                    tokio::time::timeout(WAIT, TlsConnector::from(Arc::new(cfg)).connect(name, s))
                        .await
                        .expect("handshake in time")
                        .expect("client trusting the proxy CA completes the handshake");
                Box::new(tls)
            }
        }
    }

    /// The request line for `method path` to `host`: absolute-form on the
    /// plain path, origin-form inside a tunnel.
    pub fn start_line(&self, method: &str, host: &str, path: &str) -> String {
        match self.via {
            Via::Plain => format!("{method} http://{host}:{}{path} HTTP/1.1", self.port),
            Via::Tls => format!("{method} {path} HTTP/1.1"),
        }
    }

    /// Waits until the upstream has received at least `n` bytes and
    /// returns them all.
    pub async fn received_at_least(&self, n: usize) -> Vec<u8> {
        wait_for("upstream bytes", || {
            self.received.lock().unwrap().len() >= n
        })
        .await;
        self.received.lock().unwrap().clone()
    }

    pub fn connections(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }

    /// Waits for `n` recorded exchanges, then returns them (after a short
    /// pause in which a wrongly extra one would show up).
    pub async fn exchanges(&self, n: usize) -> Vec<Exchange> {
        wait_for("recorded exchanges", || {
            self.h.sink.exchanges.lock().unwrap().len() >= n
        })
        .await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        let ex = self.h.sink.exchanges.lock().unwrap().clone();
        assert_eq!(ex.len(), n, "{ex:#?}");
        ex
    }
}

pub async fn wait_for(what: &str, mut cond: impl FnMut() -> bool) {
    tokio::time::timeout(WAIT, async {
        while !cond() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {what}"));
}

/// Reads exactly `n` bytes.
pub async fn read_n(io: &mut Box<dyn Io>, n: usize) -> Vec<u8> {
    let mut out = vec![0u8; n];
    tokio::time::timeout(WAIT, io.read_exact(&mut out))
        .await
        .expect("bytes arrived in time")
        .expect("read");
    out
}

/// Reads until EOF or an error.
pub async fn read_close(io: &mut Box<dyn Io>) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = [0u8; 8192];
    tokio::time::timeout(WAIT, async {
        loop {
            match io.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => out.extend_from_slice(&buf[..n]),
            }
        }
    })
    .await
    .expect("connection was not closed in time");
    out
}

pub fn head_bytes(m: &Message) -> Bytes {
    match m.head() {
        Head::Http1Raw { bytes, .. } => bytes.clone(),
        other => panic!("expected a raw head, got {other:?}"),
    }
}

pub fn body_bytes(m: &Message) -> Bytes {
    match m.body() {
        Body::Inline { bytes, .. } => bytes.clone(),
        Body::None => Bytes::new(),
        other => panic!("expected an inline body, got {other:?}"),
    }
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// The exchange whose request head contains `needle`.
fn find<'a>(ex: &'a [Exchange], needle: &str) -> &'a Exchange {
    ex.iter()
        .find(|e| contains(&head_bytes(e.request()), needle.as_bytes()))
        .unwrap_or_else(|| panic!("no exchange with {needle:?}"))
}

fn conn(e: &Exchange) -> &ConnectionInfo {
    e.request().connection()
}

fn ok(body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

pub async fn get_round_trip_is_byte_exact(via: Via) {
    let resp = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nX-A: b\r\n\r\nhello".to_vec();
    let rig = rig(via, vec![resp.clone()], None, &["u.test"]).await;
    let start = rig.start_line("GET", "u.test", "/p?q=1");
    let req =
        format!("{start}\r\nHost: u.test\r\nProxy-Connection: keep-alive\r\nUser-Agent: x\r\n\r\n");
    let mut c = rig.open("u.test").await;
    c.write_all(req.as_bytes()).await.unwrap();
    assert_eq!(read_n(&mut c, resp.len()).await, resp);

    let expected = b"GET /p?q=1 HTTP/1.1\r\nHost: u.test\r\nUser-Agent: x\r\n\r\n";
    assert_eq!(rig.received_at_least(expected.len()).await, expected);

    let ex = rig.exchanges(1).await;
    let request = ex[0].request();
    assert_eq!(head_bytes(request), &expected[..]);
    let diffs = request.diffs();
    for d in diffs {
        assert_eq!(&req.as_bytes()[d.span().range()], &d.before()[..], "{d:?}");
    }
    let deletes: Vec<_> = diffs
        .iter()
        .filter(|d| d.op() == ChangeOp::Delete)
        .collect();
    assert_eq!(deletes.len(), 1, "{diffs:?}");
    assert_eq!(*deletes[0].reason(), ChangeReason::HopByHop);
    assert_eq!(
        &deletes[0].before()[..],
        b"Proxy-Connection: keep-alive\r\n"
    );
    let replaces: Vec<_> = diffs
        .iter()
        .filter(|d| d.op() == ChangeOp::Replace)
        .collect();
    match via {
        Via::Plain => {
            assert_eq!(diffs.len(), 2, "{diffs:?}");
            assert_eq!(replaces.len(), 1);
            assert_eq!(
                *replaces[0].reason(),
                ChangeReason::Custom("absolute-form".into())
            );
            assert_eq!(
                replaces[0].before(),
                &format!("http://u.test:{}/p?q=1", rig.port)
            );
            assert_eq!(&replaces[0].after()[..], b"/p?q=1");
        }
        Via::Tls => assert_eq!(diffs.len(), 1, "{diffs:?}"),
    }
    let response = ex[0].response().expect("a response");
    assert_eq!(
        head_bytes(response),
        &b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nX-A: b\r\n\r\n"[..]
    );
    assert_eq!(body_bytes(response), &b"hello"[..]);
    assert_eq!(conn(&ex[0]).target_addr(), "u.test");
    assert_eq!(conn(&ex[0]).port(), rig.port);
    assert!(!conn(&ex[0]).reused());
}

pub async fn keep_alive_two_requests_same_upstream_connection(via: Via) {
    let (r1, r2) = (ok("r1"), ok("r2"));
    let rig = rig(via, vec![r1.clone(), r2.clone()], None, &["u.test"]).await;
    let mut c = rig.open("u.test").await;
    for (path, resp) in [("/1", &r1), ("/2", &r2)] {
        let req = format!(
            "{}\r\nHost: u.test\r\n\r\n",
            rig.start_line("GET", "u.test", path)
        );
        c.write_all(req.as_bytes()).await.unwrap();
        assert_eq!(&read_n(&mut c, resp.len()).await, resp);
    }
    let ex = rig.exchanges(2).await;
    assert_eq!(rig.connections(), 1);
    let (first, second) = (find(&ex, "GET /1 "), find(&ex, "GET /2 "));
    assert!(!conn(first).reused());
    assert!(conn(second).reused());
    assert_eq!(conn(first).conn_id(), conn(second).conn_id());
    assert_ne!(conn(first).conn_id(), 0);
}

pub async fn connection_close_from_server_ends_relay(via: Via) {
    let r1 = b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 2\r\n\r\nr1".to_vec();
    let rig = rig(via, vec![r1.clone(), ok("r2")], None, &["u.test"]).await;
    let mut c = rig.open("u.test").await;
    let req = format!(
        "{}\r\nHost: u.test\r\n\r\n",
        rig.start_line("GET", "u.test", "/1")
    );
    c.write_all(req.as_bytes()).await.unwrap();
    assert_eq!(read_close(&mut c).await, r1);
    let ex = rig.exchanges(1).await;
    assert_eq!(body_bytes(ex[0].response().unwrap()), &b"r1"[..]);
}

pub async fn pipelined_requests_are_served_serially_in_order(via: Via) {
    let (r1, r2) = (ok("first"), ok("second"));
    let rig = rig(via, vec![r1.clone(), r2.clone()], None, &["u.test"]).await;
    let mut c = rig.open("u.test").await;
    let req = format!(
        "{}\r\nHost: u.test\r\n\r\n{}\r\nHost: u.test\r\n\r\n",
        rig.start_line("GET", "u.test", "/1"),
        rig.start_line("GET", "u.test", "/2")
    );
    c.write_all(req.as_bytes()).await.unwrap();
    let want = [r1, r2].concat();
    assert_eq!(read_n(&mut c, want.len()).await, want);
    let expected =
        b"GET /1 HTTP/1.1\r\nHost: u.test\r\n\r\nGET /2 HTTP/1.1\r\nHost: u.test\r\n\r\n";
    assert_eq!(rig.received_at_least(expected.len()).await, expected);
    let ex = rig.exchanges(2).await;
    assert_eq!(
        body_bytes(find(&ex, "GET /1 ").response().unwrap()),
        &b"first"[..]
    );
    assert_eq!(
        body_bytes(find(&ex, "GET /2 ").response().unwrap()),
        &b"second"[..]
    );
    assert_eq!(rig.connections(), 1);
}

pub async fn request_body_content_length_forwarded_exactly(via: Via) {
    let resp = b"HTTP/1.1 201 Created\r\nContent-Length: 0\r\n\r\n".to_vec();
    let rig = rig(via, vec![resp.clone()], None, &["u.test"]).await;
    let body: Vec<u8> = (0..70 * 1024).map(|i| (i % 251) as u8).collect();
    let head = format!(
        "{}\r\nHost: u.test\r\nContent-Length: {}\r\n\r\n",
        rig.start_line("POST", "u.test", "/up"),
        body.len()
    );
    let mut c = rig.open("u.test").await;
    c.write_all(head.as_bytes()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(10)).await;
    c.write_all(&body[..30_000]).await.unwrap();
    tokio::time::sleep(Duration::from_millis(10)).await;
    c.write_all(&body[30_000..]).await.unwrap();
    assert_eq!(read_n(&mut c, resp.len()).await, resp);

    let mut expected = format!(
        "POST /up HTTP/1.1\r\nHost: u.test\r\nContent-Length: {}\r\n\r\n",
        body.len()
    )
    .into_bytes();
    expected.extend_from_slice(&body);
    let got = rig.received_at_least(expected.len()).await;
    assert!(got == expected, "upstream bytes differ");
    let ex = rig.exchanges(1).await;
    assert!(
        body_bytes(ex[0].request()) == body[..],
        "recorded body differs"
    );
}

pub async fn chunked_response_with_extensions_and_trailers_is_byte_exact(via: Via) {
    let head = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nTrailer: X-T\r\n\r\n".to_vec();
    let body = b"5;ext=1\r\nhello\r\n6\r\n world\r\n0\r\nX-T: 1\r\n\r\n".to_vec();
    let resp = [head.clone(), body.clone()].concat();
    // Three-byte fragments cut through every chunk-size line and the
    // trailer block, so the relay must re-present partial tokens.
    let rig = rig(via, vec![resp.clone()], Some(3), &["u.test"]).await;
    let mut c = rig.open("u.test").await;
    let req = format!(
        "{}\r\nHost: u.test\r\n\r\n",
        rig.start_line("GET", "u.test", "/c")
    );
    c.write_all(req.as_bytes()).await.unwrap();
    assert_eq!(read_n(&mut c, resp.len()).await, resp);

    let ex = rig.exchanges(1).await;
    let response = ex[0].response().unwrap();
    assert!(
        response
            .diagnostics()
            .iter()
            .any(|d| d.code() == DiagnosticCode::ChunkExtensionPresent),
        "{:?}",
        response.diagnostics()
    );
    assert_eq!(head_bytes(response), &head[..]);
    // The stored body is the raw framed body (the model's inline-body
    // contract); decoding it gives the payload.
    let stored = body_bytes(response);
    assert_eq!(stored, &body[..]);
    assert_eq!(decode_chunked(&head, &stored), b"hello world");
}

fn decode_chunked(head: &[u8], body: &[u8]) -> Vec<u8> {
    let mut p = Parser::response(
        &ParserConfig::default(),
        ResponseContext::for_method(b"GET"),
    );
    let all = [head, body].concat();
    let (mut off, mut out) = (0, Vec::new());
    loop {
        let input = &all[off..];
        let step = p.feed(input).unwrap();
        if let Progress::BodyData { data, .. } = &step.event {
            out.extend_from_slice(&input[data.range()]);
        }
        off += step.consumed;
        if matches!(step.event, Progress::Done | Progress::NeedMore) {
            return out;
        }
    }
}

pub async fn head_and_304_responses_are_framed_without_body(via: Via) {
    let r_head = b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\n".to_vec();
    let r_304 = b"HTTP/1.1 304 Not Modified\r\nETag: \"x\"\r\nContent-Length: 10\r\n\r\n".to_vec();
    let r_ok = ok("ok");
    let script = vec![r_head.clone(), r_304.clone(), r_ok.clone()];
    let rig = rig(via, script, None, &["u.test"]).await;
    let mut c = rig.open("u.test").await;
    for (method, path, resp) in [
        ("HEAD", "/a", &r_head),
        ("GET", "/b", &r_304),
        ("GET", "/c", &r_ok),
    ] {
        let req = format!(
            "{}\r\nHost: u.test\r\n\r\n",
            rig.start_line(method, "u.test", path)
        );
        c.write_all(req.as_bytes()).await.unwrap();
        assert_eq!(&read_n(&mut c, resp.len()).await, resp, "{method} {path}");
    }
    let ex = rig.exchanges(3).await;
    assert_eq!(rig.connections(), 1);
    assert_eq!(
        body_bytes(find(&ex, "HEAD /a ").response().unwrap()),
        Bytes::new()
    );
    assert_eq!(
        body_bytes(find(&ex, "GET /b ").response().unwrap()),
        Bytes::new()
    );
    assert_eq!(
        body_bytes(find(&ex, "GET /c ").response().unwrap()),
        &b"ok"[..]
    );
}

pub async fn http10_until_close_response_ends_connection(via: Via) {
    let resp = b"HTTP/1.0 200 OK\r\nContent-Type: text/plain\r\n\r\nbody-until-close".to_vec();
    let rig = rig(via, vec![resp.clone()], None, &["u.test"]).await;
    let mut c = rig.open("u.test").await;
    let req = format!(
        "{}\r\nHost: u.test\r\n\r\n",
        rig.start_line("GET", "u.test", "/old")
    );
    c.write_all(req.as_bytes()).await.unwrap();
    assert_eq!(read_close(&mut c).await, resp);
    let ex = rig.exchanges(1).await;
    assert_eq!(
        body_bytes(ex[0].response().unwrap()),
        &b"body-until-close"[..]
    );
}

pub async fn expect_100_continue_interim_is_forwarded_and_recorded(via: Via) {
    let interim = b"HTTP/1.1 100 Continue\r\n\r\n".to_vec();
    let resp = [interim.clone(), ok("ok")].concat();
    let rig = rig(via, vec![resp.clone()], None, &["u.test"]).await;
    let mut c = rig.open("u.test").await;
    let req = format!(
        "{}\r\nHost: u.test\r\nExpect: 100-continue\r\nContent-Length: 4\r\n\r\ndata",
        rig.start_line("POST", "u.test", "/e")
    );
    c.write_all(req.as_bytes()).await.unwrap();
    assert_eq!(read_n(&mut c, resp.len()).await, resp);
    let ex = rig.exchanges(1).await;
    let response = ex[0].response().unwrap();
    assert_eq!(response.interim().len(), 1);
    assert_eq!(&response.interim()[0][..], &interim[..]);
    assert!(head_bytes(response).starts_with(b"HTTP/1.1 200 OK\r\n"));
    assert_eq!(body_bytes(ex[0].request()), &b"data"[..]);
}

pub async fn host_header_differs_from_authority_is_forwarded_unchanged(via: Via) {
    let rig = rig(via, vec![ok("hi")], None, &["a.test"]).await;
    let mut c = rig.open("a.test").await;
    let req = format!(
        "{}\r\nHost: b.test\r\n\r\n",
        rig.start_line("GET", "a.test", "/h")
    );
    c.write_all(req.as_bytes()).await.unwrap();
    assert_eq!(read_n(&mut c, ok("hi").len()).await, ok("hi"));
    let expected = b"GET /h HTTP/1.1\r\nHost: b.test\r\n\r\n";
    assert_eq!(rig.received_at_least(expected.len()).await, expected);
    let ex = rig.exchanges(1).await;
    assert!(contains(
        &head_bytes(ex[0].request()),
        b"\r\nHost: b.test\r\n"
    ));
    assert_eq!(conn(&ex[0]).target_addr(), "a.test");
}

/// Drops every request at its head.
struct DropAll;

impl Handler for DropAll {
    fn on_request_head(&self, _req: &RawMessage, _conn: &ConnectionInfo) -> Decision {
        Decision::Drop
    }
    fn on_request_body_chunk(&self, _data: &[u8]) -> Decision {
        Decision::Forward
    }
    fn on_response_head(&self, _resp: &RawMessage, _req: &RawMessage) -> Decision {
        Decision::Forward
    }
    fn on_response_body_chunk(&self, _data: &[u8]) -> Decision {
        Decision::Forward
    }
}

pub async fn handler_drop_closes_without_forwarding(via: Via) {
    let rig = rig_with(
        via,
        vec![ok("no")],
        None,
        &["u.test"],
        loopback_config(),
        Arc::new(DropAll),
    )
    .await;
    let mut c = rig.open("u.test").await;
    let req = format!(
        "{}\r\nHost: u.test\r\n\r\n",
        rig.start_line("GET", "u.test", "/d")
    );
    c.write_all(req.as_bytes()).await.unwrap();
    assert_eq!(read_close(&mut c).await, b"");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(rig.received.lock().unwrap().is_empty());
    assert_eq!(rig.connections(), 0);
    assert!(rig.h.sink.exchanges.lock().unwrap().is_empty());
}

pub async fn pipelining_depth_over_limit_closes(via: Via) {
    let mut cfg = loopback_config();
    cfg.limits = ProxyLimits::builder().max_pipelined(2).build();
    let script = vec![ok("1"), ok("2"), ok("3")];
    let rig = rig_with(via, script, None, &["u.test"], cfg, Arc::new(NoopHandler)).await;
    let mut c = rig.open("u.test").await;
    let mut req = String::new();
    for path in ["/1", "/2", "/3"] {
        req.push_str(&format!(
            "{}\r\nHost: u.test\r\n\r\n",
            rig.start_line("GET", "u.test", path)
        ));
    }
    // One write of about 150 bytes: the relay reads up to 64 KiB at a
    // time, so it sees all three heads (one TCP segment, or one TLS record)
    // in the read that brings the first. The marker wait below is bounded.
    c.write_all(req.as_bytes()).await.unwrap();
    let got = read_close(&mut c).await;
    // The first exchange completes; the connection closes after it.
    assert_eq!(got, ok("1"));
    let ex = rig.exchanges(1).await;
    assert!(head_bytes(ex[0].request()).starts_with(b"GET /1 "));
    assert_eq!(body_bytes(ex[0].response().unwrap()), &b"1"[..]);
    let expected = b"GET /1 HTTP/1.1\r\nHost: u.test\r\n\r\n";
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(rig.received.lock().unwrap().as_slice(), expected);
    wait_for("the pipelining marker", || {
        rig.h
            .sink
            .markers
            .lock()
            .unwrap()
            .iter()
            .any(|m| m.kind() == MarkerKind::LimitExceeded && m.detail().contains("pipelining"))
    })
    .await;
}
