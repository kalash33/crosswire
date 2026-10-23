//! The proxy relay golden: every case of the crosswire-h1 fidelity corpus
//! (`corpus/h1/cases`) relayed through a CONNECT tunnel, byte-exact, with
//! its diagnostics recorded — the alpha.1 "≥ 200 byte-exact captures".
//!
//! The tunnel carries plain HTTP (the client sends the case bytes, not a
//! ClientHello, after the `200 Connection established`), so the relay runs
//! its origin-form path and the bytes are observable exactly on both
//! sides. Request cases: the client writes the case, the upstream must
//! receive exactly those bytes and answers a fixed 200. Response cases:
//! the client sends a fixed request for the case's method, the upstream
//! answers with the case bytes and closes, and the client must receive
//! exactly those bytes. Each case runs twice: written whole, and in 7-byte
//! fragments with 1 ms gaps (on both sides).

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::net::IpAddr;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::corpus::{load_cases, parse_one, Case, Parsed};
use common::relay::{body_bytes, head_bytes, wait_for};
use common::upstream::spawn_raw;
use common::{connect_raw, read_until, ProxyHarness, UpstreamWiring};
use crosswire_core::{ChangeOp, DiagnosticCode};
use crosswire_h1::Progress;
use crosswire_model::{Exchange, ExchangeState, Head, MarkerKind, Message, TunnelReason};
use crosswire_proxy::{NoopHandler, ProxyConfig, ProxyLimits};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const OK: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok";
const WAIT: Duration = Duration::from_secs(10);
/// Fragmented writes cut the first this many bytes into 7-byte pieces
/// (1 ms apart) and send the rest in 64 KiB pieces, so the 1 MiB
/// generated case does not take minutes.
const FRAGMENTED_PREFIX: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pattern {
    Whole,
    Fragmented,
}

/// How one run ended when it passed.
#[derive(Debug)]
enum Outcome {
    /// Relayed byte-exact and recorded with the expected diagnostics; the
    /// diagnostic codes on the recorded messages.
    Clean(BTreeSet<String>),
    /// An `error` case that took the documented failure path; the codes on
    /// what was recorded, plus the error when it is also a diagnostic code.
    ErrorPath(String, BTreeSet<String>),
}

async fn write_pattern(s: &mut TcpStream, bytes: &[u8], pattern: Pattern) {
    match pattern {
        Pattern::Whole => s.write_all(bytes).await.unwrap(),
        Pattern::Fragmented => {
            let split = bytes.len().min(FRAGMENTED_PREFIX);
            for piece in bytes[..split].chunks(7) {
                s.write_all(piece).await.unwrap();
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            for piece in bytes[split..].chunks(64 * 1024) {
                s.write_all(piece).await.unwrap();
            }
        }
    }
}

async fn read_exact_n(s: &mut TcpStream, n: usize) -> Vec<u8> {
    let mut out = vec![0u8; n];
    tokio::time::timeout(WAIT, s.read_exact(&mut out))
        .await
        .expect("bytes arrived in time")
        .expect("read");
    out
}

async fn read_to_close(s: &mut TcpStream) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    tokio::time::timeout(WAIT, async {
        loop {
            match s.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => out.extend_from_slice(&buf[..n]),
            }
        }
    })
    .await
    .expect("connection closed in time");
    out
}

/// A plain upstream that reads one request head, answers with `resp`
/// (written per `pattern`) and closes.
async fn spawn_responder(resp: Vec<u8>, pattern: Pattern) -> (u16, Arc<Mutex<Vec<u8>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let received = Arc::new(Mutex::new(Vec::new()));
    let rec = Arc::clone(&received);
    tokio::spawn(async move {
        while let Ok((mut tcp, _)) = listener.accept().await {
            let _ = tcp.set_nodelay(true);
            let (resp, rec) = (resp.clone(), Arc::clone(&rec));
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                loop {
                    if rec.lock().unwrap().windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                    match tcp.read(&mut buf).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => rec.lock().unwrap().extend_from_slice(&buf[..n]),
                    }
                }
                write_pattern(&mut tcp, &resp, pattern).await;
                let _ = tcp.shutdown().await;
            });
        }
    });
    (port, received)
}

/// A proxy for `case` (its parser limits) reaching the plain upstream on
/// `port` as `u.test`.
async fn proxy_for(case: &Case, port: u16) -> ProxyHarness {
    let mut cfg = ProxyConfig::new("127.0.0.1:0".parse().unwrap());
    cfg.limits = ProxyLimits::builder().parse(case.config).build();
    let wiring = UpstreamWiring {
        extra_roots: Vec::new(),
        allow: vec![("127.0.0.1".to_string(), port)],
        resolve: vec![("u.test".to_string(), IpAddr::from([127, 0, 0, 1]))],
        ..UpstreamWiring::default()
    };
    ProxyHarness::start_wired(cfg, Arc::new(NoopHandler), wiring).await
}

/// Opens a CONNECT tunnel to `u.test:port` that will carry plain HTTP.
async fn tunnel(h: &ProxyHarness, port: u16) -> TcpStream {
    let mut s = connect_raw(h.addr).await;
    let authority = format!("u.test:{port}");
    s.write_all(format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n\r\n").as_bytes())
        .await
        .unwrap();
    assert_eq!(
        read_until(&mut s, b"\r\n\r\n").await,
        b"HTTP/1.1 200 Connection established\r\n\r\n"
    );
    s
}

fn is_tunnel(e: &Exchange) -> bool {
    matches!(e.request().head(), Head::Tunnel { .. })
}

fn codes(m: &Message) -> Vec<String> {
    let mut c: Vec<String> = m
        .diagnostics()
        .iter()
        .map(|d| d.code().as_str().to_owned())
        .collect();
    c.sort();
    c.dedup();
    c
}

/// The diagnostic codes on both messages of `e`.
fn exchange_codes(e: &Exchange) -> BTreeSet<String> {
    let mut set: BTreeSet<String> = codes(e.request()).into_iter().collect();
    if let Some(r) = e.response() {
        set.extend(codes(r));
    }
    set
}

/// `error` itself when it names a diagnostic code (`invalid_chunk_size`
/// is both a fatal parse error and a catalogue code).
fn error_code(error: &str) -> BTreeSet<String> {
    DiagnosticCode::ALL
        .iter()
        .map(DiagnosticCode::as_str)
        .filter(|c| *c == error)
        .map(str::to_owned)
        .collect()
}

/// The recorded exchange whose `pick` message has head `head`.
async fn exchange_with_head(
    h: &ProxyHarness,
    head: &[u8],
    pick: fn(&Exchange) -> Option<&Message>,
) -> Exchange {
    let found = Arc::new(Mutex::new(None));
    let f = Arc::clone(&found);
    wait_for("the recorded exchange", || {
        let ex = h.sink.exchanges.lock().unwrap();
        let hit = ex
            .iter()
            .find(|e| !is_tunnel(e) && pick(e).is_some_and(|m| head_bytes(m)[..] == head[..]));
        *f.lock().unwrap() = hit.cloned();
        hit.is_some()
    })
    .await;
    let e = found.lock().unwrap().take().unwrap();
    e
}

/// Checks a recorded message against the case: head bytes, diagnostic
/// set, the raw stored body against the wire body, and the stored body
/// decoded against the expected payload.
fn check_message(case: &Case, m: &Message, wire: &Parsed) {
    let name = &case.name;
    let input = &case.input;
    assert_eq!(
        head_bytes(m)[..],
        input[..wire.head_len],
        "{name}: recorded head"
    );
    assert_eq!(codes(m), case.diagnostics, "{name}: diagnostics");
    let stored = body_bytes(m);
    assert!(
        stored[..] == input[wire.head_len..wire.end],
        "{name}: stored raw body differs from the wire body ({} vs {} bytes)",
        stored.len(),
        wire.end - wire.head_len
    );
    let mut whole = head_bytes(m).to_vec();
    whole.extend_from_slice(&stored);
    let decoded = parse_one(case.parser(), &whole)
        .unwrap_or_else(|| panic!("{name}: the recorded message does not re-parse"));
    assert_eq!(decoded.payload, wire.payload, "{name}: decoded payload");
    if let Some(body) = &case.body {
        assert!(
            decoded.payload == *body,
            "{name}: decoded body != expect body"
        );
    }
}

/// The marker kind and the error text a parse error is reported with.
fn error_report(code: &str) -> (MarkerKind, &'static str) {
    match code {
        "head_too_large" => (MarkerKind::LimitExceeded, "message head too large"),
        "line_too_long" => (MarkerKind::LimitExceeded, "line too long"),
        "too_many_headers" => (MarkerKind::LimitExceeded, "too many headers"),
        "too_many_chunks" => (MarkerKind::LimitExceeded, "too many chunks"),
        "trailer_too_large" => (MarkerKind::LimitExceeded, "trailer section too large"),
        "invalid_chunk_size" => (MarkerKind::Error, "invalid chunk size"),
        "unexpected_eof" => (MarkerKind::Error, "unexpected end of input"),
        other => panic!("no proxy behaviour defined for error {other}"),
    }
}

async fn wait_marker(h: &ProxyHarness, kind: MarkerKind, needle: &str) {
    wait_for(&format!("a {kind:?} marker with {needle:?}"), || {
        h.sink
            .markers
            .lock()
            .unwrap()
            .iter()
            .any(|m| m.kind() == kind && m.detail().contains(needle))
    })
    .await;
}

async fn run_request(case: Case, pattern: Pattern) -> Outcome {
    let name = case.name.clone();
    let msgs = if case.pipelined { 2 } else { 1 };
    let up = spawn_raw(vec![OK.to_vec(); msgs], None).await;
    let h = proxy_for(&case, up.port()).await;
    let mut c = tunnel(&h, up.port()).await;
    write_pattern(&mut c, &case.input, pattern).await;

    if let Some(code) = case.error.clone() {
        // Task 7: a generated 400, a marker, the connection closed.
        let got = read_to_close(&mut c).await;
        let text = String::from_utf8_lossy(&got).into_owned();
        assert!(
            text.starts_with("HTTP/1.1 400 Bad Request\r\n"),
            "{name}: {text}"
        );
        let (kind, what) = error_report(&code);
        assert!(
            text.contains(&format!("Crosswire could not parse the request: {what}")),
            "{name}: {text}"
        );
        wait_marker(&h, kind, &format!("could not be parsed: {what}")).await;
        // What went upstream is a prefix of the case: the relay forwards
        // only bytes the parser consumed.
        let sent = up.received.lock().unwrap().clone();
        assert!(
            case.input.starts_with(&sent),
            "{name}: upstream got {sent:?}"
        );
        let mut seen = error_code(&code);
        let path = if case.framing.is_some() {
            // The head parsed: the partial exchange is recorded as Error.
            let wire_head = head_len_of(&case);
            let ex = exchange_with_head(&h, &case.input[..wire_head], |e| Some(e.request())).await;
            assert_eq!(ex.state(), ExchangeState::Error, "{name}");
            seen.extend(exchange_codes(&ex));
            "request body: 400 + Error exchange"
        } else {
            tokio::time::sleep(Duration::from_millis(50)).await;
            assert!(h.sink.exchanges.lock().unwrap().is_empty(), "{name}");
            assert_eq!(up.connections.load(Ordering::SeqCst), 0, "{name}");
            "request head: 400, nothing recorded"
        };
        return Outcome::ErrorPath(format!("{path}; {kind:?} marker"), seen);
    }

    let want = OK.repeat(msgs);
    assert_eq!(read_exact_n(&mut c, want.len()).await, want, "{name}");
    // Inside a tunnel the relay forwards the request verbatim except for
    // `Proxy-Connection` / `Proxy-Authorization` lines (hop-by-hop to the
    // proxy), which it deletes and records as diffs.
    let (forwarded, removed) = strip_proxy_lines(&case.input);
    wait_for("the upstream bytes", || {
        up.received.lock().unwrap().len() >= forwarded.len()
    })
    .await;
    assert!(
        *up.received.lock().unwrap() == forwarded,
        "{name}: upstream bytes differ"
    );
    let mut sent = case.clone();
    sent.input = forwarded;
    let wire = parse_one(sent.parser(), &sent.input).expect("the case parses");
    if !case.pipelined {
        assert_eq!(wire.end, sent.input.len(), "{name}");
    }
    let ex = exchange_with_head(&h, &sent.input[..wire.head_len], |e| Some(e.request())).await;
    assert_ne!(ex.state(), ExchangeState::Error, "{name}");
    check_message(&sent, ex.request(), &wire);
    // The diffs are exactly the deleted lines, at their offsets in the
    // head as the client sent it, and undoing them restores that head.
    let diffs = ex.request().diffs();
    assert_eq!(diffs.len(), removed.len(), "{name}: {diffs:?}");
    for (d, (at, line)) in diffs.iter().zip(&removed) {
        assert_eq!(d.op(), ChangeOp::Delete, "{name}: {d:?}");
        assert!(d.after().is_empty(), "{name}: {d:?}");
        assert_eq!(d.before()[..], line[..], "{name}: {d:?}");
        assert_eq!(d.span().range(), *at..at + line.len(), "{name}: {d:?}");
    }
    let mut restored = head_bytes(ex.request()).to_vec();
    let mut ordered: Vec<_> = diffs.iter().collect();
    ordered.sort_by_key(|d| d.span().range().start);
    for d in ordered {
        // Ascending: everything before `start` is already the original.
        let start = d.span().range().start;
        restored.splice(start..start + d.after().len(), d.before().iter().copied());
    }
    assert_eq!(
        restored[..],
        case.input[..parse_one(case.parser(), &case.input).unwrap().head_len],
        "{name}: reversing the diffs"
    );
    let resp = ex.response().expect("a response");
    assert_eq!(head_bytes(resp)[..], OK[..OK.len() - 2], "{name}");
    let mut seen = exchange_codes(&ex);

    if case.pipelined {
        // The second message: its head, diagnostics (as crosswire-h1
        // reports them for those bytes; the corpus expectations cover the
        // first message only) and body.
        let mut second = sent.clone();
        second.input = sent.input[wire.end..].to_vec();
        second.body = None;
        let wire2 = parse_one(second.parser(), &second.input).expect("the second message parses");
        assert_eq!(wire2.end, second.input.len(), "{name}: second message");
        second.diagnostics = wire2.codes.clone();
        let ex2 =
            exchange_with_head(&h, &second.input[..wire2.head_len], |e| Some(e.request())).await;
        assert_ne!(ex2.state(), ExchangeState::Error, "{name}: second");
        check_message(&second, ex2.request(), &wire2);
        seen.extend(exchange_codes(&ex2));
    }
    Outcome::Clean(seen)
}

/// `bytes` without the head's `Proxy-Connection` and `Proxy-Authorization`
/// lines, and those lines with their offsets.
fn strip_proxy_lines(bytes: &[u8]) -> (Vec<u8>, Vec<(usize, Vec<u8>)>) {
    let head_end = bytes
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map_or(bytes.len(), |i| i + 4);
    let (mut kept, mut removed, mut at) = (Vec::new(), Vec::new(), 0);
    for line in bytes[..head_end].split_inclusive(|&b| b == b'\n') {
        let lower = line.to_ascii_lowercase();
        if lower.starts_with(b"proxy-connection:") || lower.starts_with(b"proxy-authorization:") {
            removed.push((at, line.to_vec()));
        } else {
            kept.extend_from_slice(line);
        }
        at += line.len();
    }
    kept.extend_from_slice(&bytes[head_end..]);
    (kept, removed)
}

/// The status of the case's response head, when it parses.
fn status_of(case: &Case) -> Option<u16> {
    match case.parser().feed(&case.input).ok()?.event {
        Progress::Head(m) => m.head().status_code(m.bytes()),
        _ => None,
    }
}

fn head_len_of(case: &Case) -> usize {
    case.input
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|i| i + 4)
        .expect("a head")
}

async fn run_response(case: Case, pattern: Pattern) -> Outcome {
    let name = case.name.clone();
    let (port, received) = spawn_responder(case.input.clone(), pattern).await;
    let h = proxy_for(&case, port).await;
    let mut c = tunnel(&h, port).await;
    let req = if case.method == "CONNECT" {
        "CONNECT u.test:443 HTTP/1.1\r\nHost: u.test:443\r\n\r\n".to_string()
    } else {
        format!("{} /corpus HTTP/1.1\r\nHost: u.test\r\n\r\n", case.method)
    };
    write_pattern(&mut c, req.as_bytes(), pattern).await;

    if let Some(code) = case.error.clone() {
        // Task 7: the response cannot be framed, so the client gets every
        // byte through an undelimitable tunnel.
        let got = read_to_close(&mut c).await;
        assert!(got == case.input, "{name}: client bytes differ");
        // The tunnel is recorded when both directions have ended.
        drop(c);
        let (_, what) = error_report(&code);
        wait_marker(
            &h,
            MarkerKind::Error,
            &format!("undelimitable response from u.test:{port}: {what}"),
        )
        .await;
        let wire_head = head_len_of(&case);
        let ex = exchange_with_head(&h, &case.input[..wire_head], Exchange::response).await;
        assert_eq!(ex.state(), ExchangeState::Error, "{name}");
        let mut seen = error_code(&code);
        seen.extend(exchange_codes(&ex));
        wait_for("the undelimitable tunnel record", || {
            h.sink.exchanges.lock().unwrap().iter().any(|e| {
                matches!(
                    e.request().head(),
                    Head::Tunnel {
                        reason: TunnelReason::Undelimitable,
                        ..
                    }
                )
            })
        })
        .await;
        return Outcome::ErrorPath("response: undelimitable tunnel + Error marker".into(), seen);
    }

    assert!(
        read_exact_n(&mut c, case.input.len()).await == case.input,
        "{name}: client bytes differ"
    );
    if matches!(status_of(&case), Some(100..=199)) && status_of(&case) != Some(101) {
        // An interim response: forwarded byte-exact and recorded as the
        // exchange's interim head. The upstream then closes without a
        // final response, so the client gets a generated 502.
        let rest = read_to_close(&mut c).await;
        assert!(rest.starts_with(b"HTTP/1.1 502 Bad Gateway\r\n"), "{name}");
        let found = Arc::new(Mutex::new(None));
        let f = Arc::clone(&found);
        wait_for("the exchange with the interim head", || {
            let ex = h.sink.exchanges.lock().unwrap();
            let hit = ex.iter().find(|e| {
                e.response()
                    .is_some_and(|r| r.interim().iter().any(|i| i[..] == case.input[..]))
            });
            *f.lock().unwrap() = hit.cloned();
            hit.is_some()
        })
        .await;
        let ex = found.lock().unwrap().take().unwrap();
        assert_eq!(ex.state(), ExchangeState::Error, "{name}");
        // Interim heads are recorded as bytes only, with no diagnostic
        // list, so the case's diagnostics are not checked here and add
        // nothing to the code coverage (only what the generated 502
        // exchange carries does).
        return Outcome::Clean(exchange_codes(&ex));
    }
    assert!(
        *received.lock().unwrap() == req.as_bytes(),
        "{name}: upstream got other request bytes"
    );
    drop(c);
    let wire = parse_one(case.parser(), &case.input).expect("the case parses");
    assert_eq!(wire.end, case.input.len(), "{name}");
    let ex = exchange_with_head(&h, &case.input[..wire.head_len], Exchange::response).await;
    assert_ne!(ex.state(), ExchangeState::Error, "{name}");
    check_message(&case, ex.response().unwrap(), &wire);
    Outcome::Clean(exchange_codes(&ex))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_corpus_case_relays_byte_exact_and_records_its_diagnostics() {
    let cases = load_cases();
    assert!(cases.len() >= 100, "corpus has {} cases", cases.len());
    let (mut clean, mut error_path) = (0usize, 0usize);
    let mut paths: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut skipped: Vec<String> = Vec::new();
    let mut failures: Vec<String> = Vec::new();
    let mut covered: BTreeSet<String> = BTreeSet::new();
    for case in &cases {
        if let Some(bug) = &case.known_bug {
            skipped.push(format!("{} (known_bug: {bug})", case.name));
            continue;
        }
        for pattern in [Pattern::Whole, Pattern::Fragmented] {
            let c = case.clone();
            let run = tokio::spawn(async move {
                if c.kind == "request" {
                    run_request(c, pattern).await
                } else {
                    run_response(c, pattern).await
                }
            });
            match run.await {
                Ok(Outcome::Clean(seen)) => {
                    clean += 1;
                    covered.extend(seen);
                }
                Ok(Outcome::ErrorPath(path, seen)) => {
                    error_path += 1;
                    covered.extend(seen);
                    paths
                        .entry(path)
                        .or_default()
                        .push(format!("{} ({pattern:?})", case.name));
                }
                Err(e) => {
                    let msg = e.into_panic();
                    let msg = msg
                        .downcast_ref::<String>()
                        .cloned()
                        .or_else(|| msg.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                        .unwrap_or_default();
                    failures.push(format!("{} ({pattern:?}): {msg}", case.name));
                }
            }
        }
    }
    eprintln!("corpus relay: {clean} clean byte-exact captures, {error_path} error-path runs");
    for (path, runs) in &paths {
        eprintln!("  error path {path}: {}", runs.join(", "));
    }
    eprintln!("  skipped: {}", skipped.len());
    for s in &skipped {
        eprintln!("    {s}");
    }
    assert!(
        failures.is_empty(),
        "{} runs failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert!(
        clean + error_path >= 200,
        "only {} runs (clean {clean}, error path {error_path})",
        clean + error_path
    );
    // The alpha.1 DoD: the captures cover every diagnostic code in the
    // catalogue. No code is exempt: each is reachable through the proxy.
    let missing: Vec<&str> = DiagnosticCode::ALL
        .iter()
        .map(DiagnosticCode::as_str)
        .filter(|c| !covered.contains(*c))
        .collect();
    eprintln!(
        "  codes covered ({} of {}): {}",
        covered.len(),
        DiagnosticCode::ALL.len(),
        covered.iter().cloned().collect::<Vec<_>>().join(", ")
    );
    eprintln!("  codes missing: {missing:?}");
    assert!(
        missing.is_empty(),
        "diagnostic codes not covered: {missing:?}"
    );
}
