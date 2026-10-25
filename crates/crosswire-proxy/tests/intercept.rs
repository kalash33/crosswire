//! Request intercept: holding a request, forwarding it unchanged or
//! edited, dropping it, the hold queue and timeout, the skip rules, and the
//! scope state recorded on every exchange.

mod common;

use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use bytes::Bytes;
use common::relay::wait_for;
use common::upstream::{spawn_raw, RawUpstream};
use common::{
    connect_raw, loopback_config, read_until_close, ProxyHarness, TestSink, UpstreamWiring,
};
use crosswire_core::ChangeReason;
use crosswire_h1::{Parser, ParserConfig, Progress};
use crosswire_model::{ConnectionInfo, ExchangeSink, ExchangeState, ScopeState};
use crosswire_proxy::{
    Budget, EditedBytes, Handler, Intercept, InterceptError, InterceptRules, OnTimeout, Overflow,
    ProxyLimits,
};
use crosswire_scope::{Compiled, HostPattern, Rule, Scope};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use ulid::Ulid;

/// A running proxy whose handler is a real [`Intercept`], reaching a plain
/// upstream recorded as `u.test`.
struct Rig {
    h: ProxyHarness,
    intercept: Arc<Intercept>,
    upstream: RawUpstream,
    scope: Arc<RwLock<Compiled>>,
}

/// A `200 OK` with `Connection: close`, so the client reads to EOF.
fn resp(body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

/// An absolute-form GET to `u.test:{port}{path}` (the port the proxy
/// reaches the in-process upstream on).
fn get(port: u16, path: &str) -> Vec<u8> {
    format!("GET http://u.test:{port}{path} HTTP/1.1\r\nHost: u.test\r\n\r\n").into_bytes()
}

fn empty_scope() -> Arc<RwLock<Compiled>> {
    Arc::new(RwLock::new(Scope::default().compile().unwrap()))
}

/// A scope whose only include rule is an exact host.
fn scope_including(host: &str) -> Arc<RwLock<Compiled>> {
    let scope = Scope {
        include: vec![Rule {
            host: HostPattern::Exact(host.to_string()),
            ports: None,
            scheme: None,
            path: None,
        }],
        exclude: Vec::new(),
    };
    Arc::new(RwLock::new(scope.compile().unwrap()))
}

fn plain_wiring(port: u16) -> UpstreamWiring {
    UpstreamWiring {
        extra_roots: Vec::new(),
        allow: vec![("127.0.0.1".to_string(), port)],
        resolve: vec![("u.test".to_string(), IpAddr::from([127, 0, 0, 1]))],
        deny_unknown: false,
    }
}

async fn rig(
    limits: ProxyLimits,
    rules: InterceptRules,
    scope: Arc<RwLock<Compiled>>,
    script: Vec<Vec<u8>>,
    enabled: bool,
) -> Rig {
    let upstream = spawn_raw(script, None).await;
    let budget = Arc::new(Budget::new(limits.body_budget));
    let intercept = Arc::new(Intercept::new(
        &limits,
        Arc::clone(&scope),
        rules,
        budget,
        Arc::new(TestSink::default()) as Arc<dyn ExchangeSink>,
    ));
    intercept.set_enabled(enabled);
    let mut cfg = loopback_config();
    cfg.limits = limits;
    let wiring = plain_wiring(upstream.port());
    let handler: Arc<dyn Handler> = Arc::clone(&intercept) as Arc<dyn Handler>;
    let h = ProxyHarness::start_scoped(cfg, handler, wiring, Arc::clone(&scope)).await;
    Rig {
        h,
        intercept,
        upstream,
        scope,
    }
}

impl Rig {
    fn addr(&self) -> SocketAddr {
        self.h.addr
    }

    async fn open(&self, req: &[u8]) -> TcpStream {
        let mut c = connect_raw(self.addr()).await;
        c.write_all(req).await.unwrap();
        c.flush().await.unwrap();
        c
    }

    async fn wait_held(&self, n: usize) {
        let i = Arc::clone(&self.intercept);
        wait_for(&format!("{n} held requests"), move || i.held().len() == n).await;
    }

    fn upstream_bytes(&self) -> Vec<u8> {
        self.upstream.received.lock().unwrap().clone()
    }

    async fn wait_upstream_contains(&self, needle: &[u8]) {
        let up = Arc::clone(&self.upstream.received);
        let needle = needle.to_vec();
        wait_for("the upstream to receive the request", move || {
            let got = up.lock().unwrap();
            got.windows(needle.len()).any(|w| w == needle)
        })
        .await;
    }
}

#[tokio::test]
async fn disabled_intercept_forwards_immediately() {
    let r = rig(
        ProxyLimits::default(),
        InterceptRules::default(),
        empty_scope(),
        vec![resp("hi")],
        false,
    )
    .await;
    let mut c = r.open(&get(r.upstream.port(), "/p")).await;
    let got = read_until_close(&mut c).await;
    assert!(
        String::from_utf8_lossy(&got).contains("200 OK"),
        "disabled intercept forwards and the client sees the response"
    );
    assert!(r.intercept.held().is_empty());
    r.wait_upstream_contains(b"GET /p HTTP/1.1").await;
}

#[tokio::test]
async fn held_request_is_listed_and_forwarded_unchanged() {
    let r = rig(
        ProxyLimits::default(),
        InterceptRules::default(),
        empty_scope(),
        vec![resp("hi")],
        true,
    )
    .await;
    let mut c = r.open(&get(r.upstream.port(), "/p")).await;
    r.wait_held(1).await;
    let held = r.intercept.held();
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].method, "GET");
    assert!(held[0].path.contains("/p"));
    assert!(held[0].editable, "a bodiless request is editable");
    assert_eq!(held[0].size, 0);
    assert!(
        r.upstream_bytes().is_empty(),
        "nothing reaches upstream yet"
    );

    r.intercept.forward(held[0].id, None).unwrap();
    let got = read_until_close(&mut c).await;
    assert!(String::from_utf8_lossy(&got).contains("200 OK"));
    r.wait_upstream_contains(b"GET /p HTTP/1.1\r\nHost: u.test\r\n\r\n")
        .await;
    assert!(r.intercept.held().is_empty());
}

#[tokio::test]
async fn held_request_forwarded_with_edited_head_records_changes() {
    let r = rig(
        ProxyLimits::default(),
        InterceptRules::default(),
        empty_scope(),
        vec![resp("hi")],
        true,
    )
    .await;
    let mut c = r.open(&get(r.upstream.port(), "/original")).await;
    r.wait_held(1).await;
    let id = r.intercept.held()[0].id;
    let edited = EditedBytes {
        head: Bytes::from_static(b"GET /edited HTTP/1.1\r\nHost: u.test\r\nX-Edited: yes\r\n\r\n"),
        body: None,
    };
    r.intercept.forward(id, Some(edited)).unwrap();
    let got = read_until_close(&mut c).await;
    assert!(String::from_utf8_lossy(&got).contains("200 OK"));

    // The upstream receives the edited bytes, not the original.
    r.wait_upstream_contains(b"GET /edited HTTP/1.1\r\nHost: u.test\r\nX-Edited: yes\r\n\r\n")
        .await;
    let up = String::from_utf8_lossy(&r.upstream_bytes()).into_owned();
    assert!(!up.contains("/original"), "the original path is not sent");

    let sink = Arc::clone(&r.h.sink);
    wait_for("the edited exchange to be recorded", move || {
        !sink.exchanges.lock().unwrap().is_empty()
    })
    .await;
    let exchanges = r.h.sink.exchanges.lock().unwrap();
    let diffs = exchanges[0].request().diffs();
    assert!(
        diffs
            .iter()
            .any(|d| matches!(d.reason(), ChangeReason::InterceptEdit)),
        "the recorded request carries an InterceptEdit change"
    );
}

#[tokio::test]
async fn held_bytes_exposes_original_head_and_body() {
    let r = rig(
        ProxyLimits::default(),
        InterceptRules::default(),
        empty_scope(),
        vec![resp("hi")],
        true,
    )
    .await;
    let body = b"name=value";
    let head = format!(
        "POST http://u.test:{}/submit HTTP/1.1\r\nHost: u.test\r\nContent-Length: {}\r\n\r\n",
        r.upstream.port(),
        body.len()
    );
    let mut req = head.into_bytes();
    req.extend_from_slice(body);
    let mut c = r.open(&req).await;
    r.wait_held(1).await;
    let id = r.intercept.held()[0].id;

    let bytes = r.intercept.held_bytes(id).expect("held bytes are present");
    assert!(
        bytes.head.windows(7).any(|w| w == b"/submit"),
        "the head is the original request head"
    );
    assert!(bytes.head.starts_with(b"POST "));
    assert_eq!(
        bytes.body.as_deref(),
        Some(&body[..]),
        "the buffered body is returned verbatim"
    );

    // An unknown id yields nothing.
    assert!(r.intercept.held_bytes(Ulid::new()).is_none());

    r.intercept.forward(id, None).unwrap();
    let _ = read_until_close(&mut c).await;
}

#[tokio::test]
async fn held_request_dropped_closes_client() {
    let r = rig(
        ProxyLimits::default(),
        InterceptRules::default(),
        empty_scope(),
        vec![resp("hi")],
        true,
    )
    .await;
    let mut c = r.open(&get(r.upstream.port(), "/p")).await;
    r.wait_held(1).await;
    let id = r.intercept.held()[0].id;
    Intercept::drop(&r.intercept, id).unwrap();
    let got = read_until_close(&mut c).await;
    assert!(got.is_empty(), "a dropped request closes with no response");
    assert!(r.intercept.held().is_empty());
    assert!(r.upstream_bytes().is_empty(), "nothing reaches upstream");
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        r.h.sink.exchanges.lock().unwrap().is_empty(),
        "a dropped request records no exchange"
    );
}

#[tokio::test]
async fn body_over_hold_limit_streams_through_not_editable() {
    const MIB: usize = 1024 * 1024;
    let limits = ProxyLimits::builder().intercept_hold(MIB as u64).build();
    let r = rig(
        limits,
        InterceptRules::default(),
        empty_scope(),
        vec![resp("ok")],
        true,
    )
    .await;
    let body = vec![b'z'; 5 * MIB];
    let head = format!(
        "POST http://u.test:{}/up HTTP/1.1\r\nHost: u.test\r\nContent-Length: {}\r\n\r\n",
        r.upstream.port(),
        body.len()
    );
    let mut c = connect_raw(r.addr()).await;
    let (mut rd, mut wr) = c.split();
    let writer = async {
        wr.write_all(head.as_bytes()).await.unwrap();
        wr.write_all(&body).await.unwrap();
        wr.flush().await.unwrap();
    };
    let drive = async {
        r.wait_held(1).await;
        let held = r.intercept.held();
        assert!(
            !held[0].editable,
            "a body over the hold limit is not editable"
        );
        assert_eq!(held[0].size, 0, "an unbuffered body has no held size");
        // The head is still available, but an over-limit body is not buffered.
        let bytes = r.intercept.held_bytes(held[0].id).expect("held bytes");
        assert!(!bytes.head.is_empty(), "the head is always buffered");
        assert!(
            bytes.body.is_none(),
            "an over-limit body is not buffered, so no body bytes"
        );
        // Editing a non-editable request is refused; it stays held.
        let edit = EditedBytes {
            head: Bytes::from_static(b"POST /x HTTP/1.1\r\nHost: u.test\r\n\r\n"),
            body: Some(Bytes::from_static(b"small")),
        };
        assert!(matches!(
            r.intercept.forward(held[0].id, Some(edit)),
            Err(InterceptError::NotEditable)
        ));
        assert_eq!(r.intercept.held().len(), 1, "still held after NotEditable");
        r.intercept.forward(held[0].id, None).unwrap();
    };
    let read = async {
        let got = read_until_close_split(&mut rd).await;
        assert!(String::from_utf8_lossy(&got).contains("200 OK"));
    };
    tokio::join!(writer, drive, read);
    r.wait_upstream_contains(b"POST /up HTTP/1.1").await;
    // The upstream receives the rewritten (origin-form) head plus the whole
    // streamed body.
    wait_for("the whole body to reach upstream", || {
        r.upstream_bytes().len() >= body.len()
    })
    .await;
}

#[tokio::test]
async fn hold_timeout_forwards_or_504s_per_setting() {
    // Timeout -> Forward.
    let limits = ProxyLimits::builder()
        .intercept_hold_timeout(Duration::from_millis(200))
        .build();
    let rules = InterceptRules {
        on_timeout: OnTimeout::Forward,
        ..InterceptRules::default()
    };
    let r = rig(limits, rules, empty_scope(), vec![resp("hi")], true).await;
    let mut c = r.open(&get(r.upstream.port(), "/slow")).await;
    let got = read_until_close(&mut c).await;
    assert!(
        String::from_utf8_lossy(&got).contains("200 OK"),
        "a Forward timeout forwards the held request"
    );
    r.wait_upstream_contains(b"GET /slow HTTP/1.1").await;
    assert!(r.intercept.held().is_empty());

    // Timeout -> Respond504.
    let limits = ProxyLimits::builder()
        .intercept_hold_timeout(Duration::from_millis(200))
        .build();
    let rules = InterceptRules {
        on_timeout: OnTimeout::Respond504,
        ..InterceptRules::default()
    };
    let r = rig(limits, rules, empty_scope(), vec![resp("hi")], true).await;
    let mut c = r.open(&get(r.upstream.port(), "/slow")).await;
    let got = read_until_close(&mut c).await;
    assert!(
        String::from_utf8_lossy(&got).contains("504 Gateway Timeout"),
        "a Respond504 timeout answers the client with a 504"
    );
    assert!(r.upstream_bytes().is_empty(), "nothing reaches upstream");
    let sink = Arc::clone(&r.h.sink);
    wait_for("the 504 exchange to be recorded", move || {
        !sink.exchanges.lock().unwrap().is_empty()
    })
    .await;
    let exchanges = r.h.sink.exchanges.lock().unwrap();
    assert_eq!(exchanges[0].state(), ExchangeState::Error);
}

#[tokio::test]
async fn queue_overflow_forwards_per_setting() {
    // ForwardOldest: the third hold evicts and forwards the oldest.
    let limits = ProxyLimits::builder().intercept_queue(2).build();
    let rules = InterceptRules {
        overflow: Overflow::ForwardOldest,
        ..InterceptRules::default()
    };
    let r = rig(limits, rules, empty_scope(), vec![resp("x"); 8], true).await;
    let _c1 = r.open(&get(r.upstream.port(), "/1")).await;
    r.wait_held(1).await;
    let _c2 = r.open(&get(r.upstream.port(), "/2")).await;
    r.wait_held(2).await;
    let _c3 = r.open(&get(r.upstream.port(), "/3")).await;
    // The oldest is forwarded; the queue holds /2 and /3.
    wait_for("the oldest to be forwarded", || {
        let held = r.intercept.held();
        held.len() == 2 && held.iter().all(|h| !h.path.contains("/1"))
    })
    .await;
    r.wait_upstream_contains(b"GET /1 HTTP/1.1").await;

    // ForwardNewest: the third hold is forwarded without being held.
    let limits = ProxyLimits::builder().intercept_queue(2).build();
    let rules = InterceptRules {
        overflow: Overflow::ForwardNewest,
        ..InterceptRules::default()
    };
    let r = rig(limits, rules, empty_scope(), vec![resp("x"); 8], true).await;
    let _c1 = r.open(&get(r.upstream.port(), "/1")).await;
    r.wait_held(1).await;
    let _c2 = r.open(&get(r.upstream.port(), "/2")).await;
    r.wait_held(2).await;
    let mut c3 = r.open(&get(r.upstream.port(), "/3")).await;
    // /3 is forwarded straight away; /1 and /2 stay held.
    let got = read_until_close(&mut c3).await;
    assert!(String::from_utf8_lossy(&got).contains("200 OK"));
    r.wait_upstream_contains(b"GET /3 HTTP/1.1").await;
    let held = r.intercept.held();
    assert_eq!(held.len(), 2);
    assert!(held.iter().any(|h| h.path.contains("/1")));
    assert!(held.iter().any(|h| h.path.contains("/2")));
}

#[tokio::test]
async fn skip_rules_apply_extension_and_out_of_scope() {
    // A skipped extension is never held.
    let r = rig(
        ProxyLimits::default(),
        InterceptRules::default(),
        empty_scope(),
        vec![resp("img")],
        true,
    )
    .await;
    let mut c = r.open(&get(r.upstream.port(), "/logo.png")).await;
    let got = read_until_close(&mut c).await;
    assert!(String::from_utf8_lossy(&got).contains("200 OK"));
    assert!(r.intercept.held().is_empty(), ".png is skipped, not held");
    r.wait_upstream_contains(b"GET /logo.png HTTP/1.1").await;

    // Under a scope that excludes u.test, an out-of-scope request is skipped;
    // under an empty scope it is held.
    let rules = InterceptRules {
        skip_out_of_scope: true,
        ..InterceptRules::default()
    };
    let r = rig(
        ProxyLimits::default(),
        rules,
        scope_including("other.test"),
        vec![resp("a"), resp("b")],
        true,
    )
    .await;
    let mut c = r.open(&get(r.upstream.port(), "/p")).await;
    let got = read_until_close(&mut c).await;
    assert!(String::from_utf8_lossy(&got).contains("200 OK"));
    assert!(
        r.intercept.held().is_empty(),
        "an out-of-scope request is skipped"
    );

    // Swap in an empty scope: now the request is held.
    *r.scope.write().unwrap() = Scope::default().compile().unwrap();
    let _c = r.open(&get(r.upstream.port(), "/q")).await;
    r.wait_held(1).await;
    assert!(r.intercept.held()[0].path.contains("/q"));
}

#[tokio::test]
async fn default_rules_skip_out_of_scope() {
    // Spec §4.6: the default rules skip out-of-scope traffic. Under a scope
    // that includes u.test, an out-of-scope request is forwarded while an
    // in-scope one is held.
    let r = rig(
        ProxyLimits::default(),
        InterceptRules::default(),
        scope_including("u.test"),
        vec![resp("a"), resp("b")],
        true,
    )
    .await;
    assert!(
        InterceptRules::default().skip_out_of_scope,
        "the default skips out-of-scope traffic"
    );

    // holds() is the gate the relay consults; it skips Out, holds In, and is
    // a no-op under an empty scope (Empty, not Out).
    let mut p = Parser::request(&ParserConfig::default());
    let Progress::Head(head) = p
        .feed(b"GET /p HTTP/1.1\r\nHost: u.test\r\n\r\n")
        .unwrap()
        .event
    else {
        panic!("a head");
    };
    let conn = ConnectionInfo::new("u.test", 80);
    assert!(!r.intercept.holds(&head, &conn, ScopeState::Out));
    assert!(r.intercept.holds(&head, &conn, ScopeState::In));
    assert!(r.intercept.holds(&head, &conn, ScopeState::Empty));

    // End to end: an in-scope request is held.
    let _in = r.open(&get(r.upstream.port(), "/in")).await;
    r.wait_held(1).await;
    assert!(r.intercept.held()[0].path.contains("/in"));

    // An out-of-scope request (scope now excludes the host) is forwarded,
    // never held.
    *r.scope.write().unwrap() = including("other.test");
    let mut out = r.open(&get(r.upstream.port(), "/out")).await;
    let got = read_until_close(&mut out).await;
    assert!(String::from_utf8_lossy(&got).contains("200 OK"));
    assert_eq!(
        r.intercept.held().len(),
        1,
        "the out-of-scope request was forwarded, not held"
    );
    r.wait_upstream_contains(b"GET /out HTTP/1.1").await;
}

#[tokio::test]
async fn forward_all_releases_everything() {
    let r = rig(
        ProxyLimits::default(),
        InterceptRules::default(),
        empty_scope(),
        vec![resp("x"); 8],
        true,
    )
    .await;
    let mut c1 = r.open(&get(r.upstream.port(), "/1")).await;
    let mut c2 = r.open(&get(r.upstream.port(), "/2")).await;
    let mut c3 = r.open(&get(r.upstream.port(), "/3")).await;
    r.wait_held(3).await;
    r.intercept.forward_all();
    for c in [&mut c1, &mut c2, &mut c3] {
        let got = read_until_close(c).await;
        assert!(String::from_utf8_lossy(&got).contains("200 OK"));
    }
    assert!(r.intercept.held().is_empty());
    wait_for("all three to reach upstream", || {
        r.upstream.connections.load(Ordering::SeqCst) >= 3
    })
    .await;
}

#[tokio::test]
async fn scope_state_is_recorded_on_history_exchanges() {
    let r = rig(
        ProxyLimits::default(),
        InterceptRules::default(),
        empty_scope(),
        vec![resp("x"); 8],
        false,
    )
    .await;

    // Empty scope -> Empty.
    one_request(&r, &get(r.upstream.port(), "/a"), "Host: u.test").await;
    assert_eq!(last_scope(&r), ScopeState::Empty);

    // In-scope host -> In.
    *r.scope.write().unwrap() = including("u.test");
    one_request(&r, &get(r.upstream.port(), "/b"), "Host: u.test").await;
    assert_eq!(last_scope(&r), ScopeState::In);

    // Scope excludes the host -> Out.
    *r.scope.write().unwrap() = including("other.test");
    one_request(&r, &get(r.upstream.port(), "/c"), "Host: u.test").await;
    assert_eq!(last_scope(&r), ScopeState::Out);

    // Connection in scope, Host header out -> HostMismatch.
    *r.scope.write().unwrap() = including("u.test");
    let req = format!(
        "GET http://u.test:{}/d HTTP/1.1\r\nHost: evil.test\r\n\r\n",
        r.upstream.port()
    );
    one_request(&r, req.as_bytes(), "Host: evil.test").await;
    assert_eq!(last_scope(&r), ScopeState::HostMismatch);
}

fn including(host: &str) -> Compiled {
    Scope {
        include: vec![Rule {
            host: HostPattern::Exact(host.to_string()),
            ports: None,
            scheme: None,
            path: None,
        }],
        exclude: Vec::new(),
    }
    .compile()
    .unwrap()
}

/// Sends one request, reads to EOF, and waits for one more recorded exchange.
async fn one_request(r: &Rig, req: &[u8], _host: &str) {
    let before = r.h.sink.exchanges.lock().unwrap().len();
    let mut c = connect_raw(r.addr()).await;
    c.write_all(req).await.unwrap();
    c.flush().await.unwrap();
    let _ = read_until_close(&mut c).await;
    let sink = Arc::clone(&r.h.sink);
    wait_for("one more recorded exchange", move || {
        sink.exchanges.lock().unwrap().len() > before
    })
    .await;
}

fn last_scope(r: &Rig) -> ScopeState {
    r.h.sink
        .exchanges
        .lock()
        .unwrap()
        .last()
        .unwrap()
        .scope_state()
}

/// Reads one half of a split stream to EOF.
async fn read_until_close_split(rd: &mut tokio::net::tcp::ReadHalf<'_>) -> Vec<u8> {
    use tokio::io::AsyncReadExt;
    let mut out = Vec::new();
    let mut buf = [0u8; 8192];
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match rd.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => out.extend_from_slice(&buf[..n]),
            }
        }
    })
    .await
    .expect("connection closed in time");
    out
}
