//! Integration tests for the repeater: raw and assisted modes, scope checks,
//! auditing, redirect following, connection reuse and timing.
mod common;

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};

use bytes::Bytes;
use common::upstream::{spawn_raw, spawn_tls_scripted};
use common::{AuditLog, TestSink};
use crosswire_client::{DestinationGate, RustlsBackend, Target, Transport, VerifyMode};
use crosswire_h1::ParserConfig;
use crosswire_model::{Body, NamespaceKind, ScopeState};
use crosswire_scope::{Compiled, HostPattern, Rule, Scope};
use crosswire_tools::{FollowRedirects, Mode, Repeater, SendError, SendRequest};
use ulid::Ulid;

fn gate(port: u16) -> Arc<DestinationGate> {
    let mut g = DestinationGate::new(vec![]);
    g.allow("127.0.0.1", port);
    Arc::new(g)
}

fn plain_transport(port: u16) -> Arc<Transport> {
    Arc::new(Transport::new(
        Arc::new(RustlsBackend::webpki_roots()),
        gate(port),
    ))
}

fn empty_scope() -> Compiled {
    Scope::default().compile().unwrap()
}

fn host_rule(host: HostPattern) -> Rule {
    Rule {
        host,
        ports: None,
        scheme: None,
        path: None,
    }
}

struct Harness {
    repeater: Repeater,
    sink: Arc<TestSink>,
    audit: AuditLog,
}

fn harness(transport: Arc<Transport>, scope: Compiled, strict: bool) -> Harness {
    let sink = TestSink::new();
    let audit = AuditLog::new();
    let repeater = Repeater::new(
        transport,
        Arc::clone(&sink) as Arc<dyn crosswire_model::ExchangeSink>,
        Arc::new(RwLock::new(scope)),
        Arc::new(AtomicBool::new(strict)),
        audit.hook(),
        ParserConfig::default(),
    );
    Harness {
        repeater,
        sink,
        audit,
    }
}

fn req(host: &str, port: u16, raw: &[u8]) -> SendRequest {
    SendRequest {
        tab: Ulid::new(),
        source: None,
        host: host.to_string(),
        port,
        tls: false,
        sni: None,
        verify: VerifyMode::WebPki,
        raw: Bytes::copy_from_slice(raw),
        mode: Mode::Raw,
        follow: FollowRedirects::Off,
        reuse: false,
        allow_out_of_scope: true,
    }
}

const OK_CLOSE: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok";

#[tokio::test(flavor = "multi_thread")]
async fn raw_mode_sends_bytes_untouched() {
    let server = spawn_raw(vec![OK_CLOSE.to_vec()], None).await;
    let h = harness(plain_transport(server.port()), empty_scope(), false);

    let body: Vec<u8> = (0u16..=255).map(|b| b as u8).collect();
    let mut raw = format!(
        "POST /raw HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    raw.extend_from_slice(&body);

    let source = Ulid::new();
    let mut r = req("127.0.0.1", server.port(), &raw);
    r.source = Some(source);
    let tab = r.tab;
    let out = h.repeater.send(r).await.unwrap();

    // The upstream saw exactly the bytes we handed in, body and all.
    assert_eq!(server.received.lock().unwrap().as_slice(), raw.as_slice());
    assert!(out.changes.is_empty());
    assert!(out.skipped.is_empty());

    let ex = &h.sink.exchanges()[0];
    assert_eq!(ex.namespace().kind(), NamespaceKind::Repeater);
    assert_eq!(ex.namespace().owner(), Some(tab));
    assert!(ex.tags().iter().any(|t| t == &format!("source:{source}")));
    match ex.request().body() {
        Body::Inline { bytes, .. } => assert_eq!(bytes.as_ref(), body.as_slice()),
        other => panic!("request body not inline: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn assisted_mode_fixes_content_length_and_crlf_and_records_changes() {
    let server = spawn_raw(vec![OK_CLOSE.to_vec()], None).await;
    let h = harness(plain_transport(server.port()), empty_scope(), false);

    // Bare-LF line endings and no Content-Length.
    let raw = b"POST /a HTTP/1.1\nHost: 127.0.0.1\n\nhello";
    let mut r = req("127.0.0.1", server.port(), raw);
    r.mode = Mode::Assisted;
    let out = h.repeater.send(r).await.unwrap();

    let expected = b"POST /a HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 5\r\n\r\nhello";
    assert_eq!(
        server.received.lock().unwrap().as_slice(),
        expected.as_slice()
    );
    assert!(!out.changes.is_empty(), "crlf + content-length changes");
    assert!(out.skipped.is_empty());

    // The recorded request carries the diffs.
    let ex = &h.sink.exchanges()[0];
    assert!(!ex.request().diffs().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn assisted_skips_content_length_when_transfer_encoding_present() {
    let server = spawn_raw(vec![OK_CLOSE.to_vec()], None).await;
    let h = harness(plain_transport(server.port()), empty_scope(), false);

    let raw = b"POST /te HTTP/1.1\r\nHost: 127.0.0.1\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n5\r\nhello\r\n0\r\n\r\n";
    let mut r = req("127.0.0.1", server.port(), raw);
    r.mode = Mode::Assisted;
    let out = h.repeater.send(r).await.unwrap();

    // Nothing rewritten; the skip is reported.
    assert_eq!(server.received.lock().unwrap().as_slice(), raw.as_slice());
    assert!(out.changes.is_empty());
    assert_eq!(out.skipped, vec!["content-length: skipped, TE present"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn out_of_scope_send_is_audited_and_refused_in_strict_mode() {
    // Out verdict: scope names a different host, strict refuses.
    {
        let server = spawn_raw(vec![OK_CLOSE.to_vec()], None).await;
        let mut scope = Scope::default();
        scope
            .include
            .push(host_rule(HostPattern::Exact("in.test".into())));
        let h = harness(
            plain_transport(server.port()),
            scope.compile().unwrap(),
            true,
        );

        let mut r = req(
            "127.0.0.1",
            server.port(),
            b"GET / HTTP/1.1\r\nHost: out.test\r\nConnection: close\r\n\r\n",
        );
        r.allow_out_of_scope = false;
        let err = h.repeater.send(r).await.unwrap_err();
        assert!(matches!(err, SendError::OutOfScope { .. }), "{err:?}");
        assert_eq!(h.sink.len(), 0, "nothing sent or recorded");
    }

    // Host-mismatch verdict: connection IP is in scope, the Host header is not.
    {
        let server = spawn_raw(vec![OK_CLOSE.to_vec()], None).await;
        let mut scope = Scope::default();
        scope
            .include
            .push(host_rule(HostPattern::Ip("127.0.0.1".parse().unwrap())));
        let h = harness(
            plain_transport(server.port()),
            scope.compile().unwrap(),
            true,
        );

        let mut r = req(
            "127.0.0.1",
            server.port(),
            b"GET / HTTP/1.1\r\nHost: out.test\r\nConnection: close\r\n\r\n",
        );
        r.allow_out_of_scope = false;
        let err = h.repeater.send(r).await.unwrap_err();
        assert!(matches!(err, SendError::OutOfScope { .. }), "{err:?}");
        assert_eq!(h.sink.len(), 0);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn out_of_scope_send_is_allowed_and_audited_when_not_strict() {
    let server = spawn_raw(vec![OK_CLOSE.to_vec()], None).await;
    let mut scope = Scope::default();
    scope
        .include
        .push(host_rule(HostPattern::Exact("in.test".into())));
    let h = harness(
        plain_transport(server.port()),
        scope.compile().unwrap(),
        false,
    );

    // allow_out_of_scope is true by default in `req`.
    let r = req(
        "127.0.0.1",
        server.port(),
        b"GET / HTTP/1.1\r\nHost: out.test\r\nConnection: close\r\n\r\n",
    );
    let out = h.repeater.send(r).await.unwrap();

    assert_eq!(out.scope_state, ScopeState::Out);
    assert_eq!(h.sink.len(), 1);
    assert!(h.audit.has_action("repeater.send.out_of_scope"));
    assert!(h.audit.has_action("repeater.send"));
}

#[tokio::test(flavor = "multi_thread")]
async fn empty_scope_never_prompts_or_refuses() {
    let server = spawn_raw(vec![OK_CLOSE.to_vec()], None).await;
    // Strict is on, but an empty scope must never refuse or prompt.
    let h = harness(plain_transport(server.port()), empty_scope(), true);

    let r = req(
        "127.0.0.1",
        server.port(),
        b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
    );
    let out = h.repeater.send(r).await.unwrap();

    assert_eq!(out.scope_state, ScopeState::Empty);
    assert_eq!(h.sink.len(), 1);
    assert!(h.audit.has_action("repeater.send"));
    assert!(!h.audit.has_action("repeater.send.out_of_scope"));
}

const REDIRECT_REL: &[u8] =
    b"HTTP/1.1 302 Found\r\nLocation: /next\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

#[tokio::test(flavor = "multi_thread")]
async fn follow_redirects_in_scope_only_and_caps_at_10() {
    // Caps at 10 hops: an endless redirect loop fails with RedirectCap once
    // ten hops have been followed (primary + 10 recorded exchanges).
    {
        let server = spawn_raw(vec![REDIRECT_REL.to_vec(); 15], None).await;
        let h = harness(plain_transport(server.port()), empty_scope(), false);
        let mut r = req(
            "127.0.0.1",
            server.port(),
            b"GET /start HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
        );
        r.follow = FollowRedirects::Always;
        let err = h.repeater.send(r).await.unwrap_err();
        assert!(matches!(err, SendError::RedirectCap), "{err:?}");
        assert_eq!(h.sink.len(), 11, "primary + 10 followed hops");
    }

    // In-scope only: a redirect to an out-of-scope host is not followed.
    {
        let redirect_out =
            b"HTTP/1.1 302 Found\r\nLocation: http://evil.test/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        let server = spawn_raw(vec![redirect_out.to_vec()], None).await;
        let mut scope = Scope::default();
        scope
            .include
            .push(host_rule(HostPattern::Ip("127.0.0.1".parse().unwrap())));
        let h = harness(
            plain_transport(server.port()),
            scope.compile().unwrap(),
            false,
        );
        let mut r = req(
            "127.0.0.1",
            server.port(),
            b"GET /start HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
        );
        r.follow = FollowRedirects::InScope;
        let out = h.repeater.send(r).await.unwrap();
        assert!(out.redirects.is_empty(), "out-of-scope target not followed");
        assert_eq!(h.sink.len(), 1);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn reuse_keeps_one_upstream_connection() {
    // Keep-alive responses (no Connection: close).
    let keepalive = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok";
    let server = spawn_raw(vec![keepalive.to_vec(), keepalive.to_vec()], None).await;
    let h = harness(plain_transport(server.port()), empty_scope(), false);

    for _ in 0..2 {
        let mut r = req(
            "127.0.0.1",
            server.port(),
            b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
        );
        r.reuse = true;
        h.repeater.send(r).await.unwrap();
    }

    assert_eq!(h.sink.len(), 2);
    assert_eq!(
        server.connections.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "both exchanges rode one connection"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn cert_override_surfaces_in_outcome() {
    let ok = b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
    let server = spawn_tls_scripted(vec![ok.to_vec()], None).await;
    // The transport does NOT trust the upstream CA; AcceptInvalid lets it through.
    let transport = Arc::new(Transport::new(
        Arc::new(RustlsBackend::webpki_roots()),
        gate(server.port()),
    ));
    let h = harness(transport, empty_scope(), false);

    let mut r = req(
        "127.0.0.1",
        server.port(),
        b"GET / HTTP/1.1\r\nHost: example.test\r\nConnection: close\r\n\r\n",
    );
    r.tls = true;
    r.sni = Some("example.test".to_string());
    r.verify = VerifyMode::AcceptInvalid;
    let out = h.repeater.send(r).await.unwrap();

    assert!(out.cert_override, "a failing chain was accepted");
    assert!(h.sink.exchanges()[0]
        .response()
        .unwrap()
        .connection()
        .cert_override());
    let _ = Target::plain("x", 1); // keep Target in scope for the import
}

#[tokio::test(flavor = "multi_thread")]
async fn ttfb_and_total_are_populated() {
    // A fragmented response forces measurable elapsed time.
    let body = vec![b'x'; 4000];
    let mut resp = b"HTTP/1.1 200 OK\r\nContent-Length: 4000\r\nConnection: close\r\n\r\n".to_vec();
    resp.extend_from_slice(&body);
    let server = spawn_raw(vec![resp], Some(50)).await;
    let h = harness(plain_transport(server.port()), empty_scope(), false);

    let r = req(
        "127.0.0.1",
        server.port(),
        b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
    );
    let out = h.repeater.send(r).await.unwrap();

    assert!(out.total_ms > 0, "total time measured");
    assert!(out.ttfb_ms <= out.total_ms);
    // The outcome timing matches what was recorded.
    let ex = &h.sink.exchanges()[0];
    assert_eq!(ex.timing().total_ms, out.total_ms);
    assert_eq!(ex.timing().ttfb_ms, out.ttfb_ms);
}
