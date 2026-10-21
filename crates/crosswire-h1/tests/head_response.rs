//! Tests for response status-line parsing.
use crosswire_core::DiagnosticCode;
use crosswire_h1::*;

fn ctx() -> ResponseContext {
    ResponseContext::for_method(b"GET")
}

fn parse(b: &[u8]) -> RawMessage {
    let mut p = Parser::response(&ParserConfig::default(), ctx());
    match p.feed(b).unwrap().event {
        Progress::Head(m) => m,
        other => panic!("expected head, got {other:?}"),
    }
}

fn has(m: &RawMessage, c: DiagnosticCode) -> bool {
    m.diagnostics().iter().any(|d| d.code() == c)
}

fn parts<'a>(m: &RawMessage, raw: &'a [u8]) -> (&'a [u8], &'a [u8], &'a [u8]) {
    let StartLine::Response {
        version,
        status,
        reason,
    } = m.head().start()
    else {
        panic!("not a response start line")
    };
    (
        &raw[version.range()],
        &raw[status.range()],
        &raw[reason.range()],
    )
}

#[test]
fn normal_status_line() {
    let raw = b"HTTP/1.1 404 Not Found\r\nX-A: 1\r\n\r\n";
    let m = parse(raw);
    assert_eq!(m.kind(), Kind::Response);
    assert_eq!(
        parts(&m, raw),
        (&b"HTTP/1.1"[..], &b"404"[..], &b"Not Found"[..])
    );
    assert_eq!(m.head().status_code(raw), Some(404));
    assert!(m.diagnostics().is_empty());
    assert!(matches!(m.head().framing(), BodyFraming::None));
}

#[test]
fn empty_reason_with_trailing_space() {
    let raw = b"HTTP/1.1 200 \r\n\r\n";
    let m = parse(raw);
    assert_eq!(parts(&m, raw), (&b"HTTP/1.1"[..], &b"200"[..], &b""[..]));
    assert_eq!(m.head().status_code(raw), Some(200));
    assert!(!has(&m, DiagnosticCode::VersionUnusual));
}

#[test]
fn missing_reason_and_space() {
    let raw = b"HTTP/1.1 200\r\n\r\n";
    let m = parse(raw);
    assert_eq!(parts(&m, raw), (&b"HTTP/1.1"[..], &b"200"[..], &b""[..]));
    assert_eq!(m.head().status_code(raw), Some(200));
    assert!(m.diagnostics().is_empty());
}

#[test]
fn non_numeric_status() {
    let raw = b"HTTP/1.1 abc x\r\n\r\n";
    let m = parse(raw);
    assert_eq!(m.head().status_code(raw), None);
    assert!(has(&m, DiagnosticCode::VersionUnusual));
}

#[test]
fn bad_status_digit_counts() {
    for raw in [
        &b"HTTP/1.1 20 x\r\n\r\n"[..],
        b"HTTP/1.1 2000 x\r\n\r\n",
        b"HTTP/1.1\r\n\r\n",
        b"HTTP/1.1 \r\n\r\n",
        b"\xff\xff\r\n\r\n",
    ] {
        let m = parse(raw);
        assert_eq!(m.head().status_code(raw), None, "{raw:?}");
        assert!(has(&m, DiagnosticCode::VersionUnusual), "{raw:?}");
    }
}

#[test]
fn unusual_version() {
    let raw = b"HTTP/2.0 200 OK\r\n\r\n";
    let m = parse(raw);
    assert!(has(&m, DiagnosticCode::VersionUnusual));
    assert_eq!(m.head().status_code(raw), Some(200));
}

#[test]
fn needmore_split_inside_status_line() {
    let raw = b"HTTP/1.1 200 OK\r\nX-A: 1\r\n\r\n";
    let whole = parse(raw);
    let mut p = Parser::response(&ParserConfig::default(), ctx());
    let (a, b) = raw.split_at(10);
    assert!(matches!(p.feed(a).unwrap().event, Progress::NeedMore));
    let Progress::Head(m) = p.feed(b).unwrap().event else {
        panic!()
    };
    assert_eq!(m.head().start(), whole.head().start());
    assert_eq!(m.bytes().as_ref(), raw);
}

#[test]
fn request_only_diagnostics_not_emitted() {
    let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n";
    let m = parse(raw);
    assert!(!has(&m, DiagnosticCode::MissingHost));
    let raw = b"HTTP/1.1 200 http://x/\r\nHost: a\r\nHost: b\r\n\r\n";
    let m = parse(raw);
    for c in [
        DiagnosticCode::MissingHost,
        DiagnosticCode::DuplicateHost,
        DiagnosticCode::AbsoluteFormTarget,
        DiagnosticCode::Http09Request,
    ] {
        assert!(!has(&m, c), "{c:?}");
    }
}

#[test]
fn header_diagnostics_apply_to_responses() {
    let raw = b"HTTP/1.1 200 OK\nx-a : 1\n\n";
    let m = parse(raw);
    assert!(has(&m, DiagnosticCode::BareLf));
    assert!(has(&m, DiagnosticCode::WhitespaceBeforeColon));
    assert!(has(&m, DiagnosticCode::NonCanonicalHeaderCase));
}

#[test]
fn start_target_range() {
    let raw = b"HTTP/1.1 200 OK\r\n\r\n";
    assert_eq!(parse(raw).head().start_target_range(), None);
    const REQ: &[u8] = b"GET /a?b=1 HTTP/1.1\r\nHost: x.io\r\nX-A: 1\r\n\r\n";
    let mut p = Parser::request(&ParserConfig::default());
    let Progress::Head(m) = p.feed(REQ).unwrap().event else {
        panic!()
    };
    let r = m.head().start_target_range().unwrap();
    assert_eq!(&REQ[r], b"/a?b=1");
}

#[test]
fn status_code_on_wrong_buffer_is_none() {
    let raw = b"HTTP/1.1 200 OK\r\n\r\n";
    assert_eq!(parse(raw).head().status_code(b"HT"), None);
}
