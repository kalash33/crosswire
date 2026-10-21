//! Lenient head diagnostics: odd input is accepted and annotated.
use crosswire_core::{Diagnostic, DiagnosticCode, Profile, Severity};
use crosswire_h1::*;

fn parse(b: &[u8], profile: Profile) -> RawMessage {
    let cfg = ParserConfig::builder().profile(profile).build();
    match Parser::request(&cfg).feed(b).unwrap().event {
        Progress::Head(m) => m,
        other => panic!("expected head, got {other:?}"),
    }
}

fn has(m: &RawMessage, c: DiagnosticCode) -> Option<&Diagnostic> {
    m.diagnostics().iter().find(|d| d.code() == c)
}

fn count(m: &RawMessage, c: DiagnosticCode) -> usize {
    m.diagnostics().iter().filter(|d| d.code() == c).count()
}

fn text<'a>(raw: &'a [u8], d: &Diagnostic) -> &'a [u8] {
    &raw[d.span().range()]
}

#[test]
fn clean_head_has_no_diagnostics() {
    let m = parse(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n", Profile::Strict);
    assert!(m.diagnostics().is_empty());
}

#[test]
fn bare_lf_terminators() {
    let m = parse(b"GET / HTTP/1.1\nHost: x\n\n", Profile::Lenient);
    let d = has(&m, DiagnosticCode::BareLf).unwrap();
    assert_eq!(d.severity(), Severity::Warning);
    assert_eq!(m.head().headers().len(), 1);
    assert_eq!(count(&m, DiagnosticCode::BareLf), 3); // one per bare terminator
    let m = parse(b"GET / HTTP/1.1\nHost: x\n\n", Profile::Strict);
    assert_eq!(
        has(&m, DiagnosticCode::BareLf).unwrap().severity(),
        Severity::Error
    );
}

#[test]
fn mixed_terminators_end_the_head() {
    for raw in [
        &b"GET / HTTP/1.1\r\nHost: x\r\n\n"[..],
        b"GET / HTTP/1.1\r\nHost: x\n\r\n",
    ] {
        let m = parse(raw, Profile::Lenient);
        assert_eq!(m.bytes().as_ref(), raw);
        assert_eq!(count(&m, DiagnosticCode::BareLf), 1);
    }
}

#[test]
fn bare_cr_inside_line() {
    let raw = b"GET / HTTP/1.1\r\nHost: a\rb\r\n\r\n";
    let m = parse(raw, Profile::Lenient);
    let d = has(&m, DiagnosticCode::BareCr).unwrap();
    assert_eq!(text(raw, d), b"\r");
    assert_eq!(&raw[m.head().headers()[0].value().range()], b"a\rb");
}

#[test]
fn obs_fold_keeps_bytes() {
    let raw = b"GET / HTTP/1.1\r\nHost: x\r\nX-A: 1\r\n 2\r\n\r\n";
    let m = parse(raw, Profile::Lenient);
    assert!(has(&m, DiagnosticCode::ObsFold).is_some());
    assert_eq!(m.head().headers().len(), 2);
    assert_eq!(&raw[m.head().headers()[1].value().range()], b"1\r\n 2");
    assert_eq!(m.bytes().as_ref(), raw);
    let m = parse(raw, Profile::Strict);
    assert_eq!(
        has(&m, DiagnosticCode::ObsFold).unwrap().severity(),
        Severity::Error
    );
}

#[test]
fn whitespace_before_colon_and_trailing_space() {
    let m = parse(b"GET / HTTP/1.1\r\nHost : x\r\n\r\n", Profile::Strict);
    assert_eq!(
        has(&m, DiagnosticCode::WhitespaceBeforeColon)
            .unwrap()
            .severity(),
        Severity::Error
    );
    assert_eq!(
        has(&m, DiagnosticCode::TrailingSpaceInName)
            .unwrap()
            .severity(),
        Severity::Error
    );
    assert!(has(&m, DiagnosticCode::MissingHost).is_none());
}

#[test]
fn leading_whitespace_is_skipped_and_spanned() {
    let raw = b"\r\n \tGET / HTTP/1.1\r\nHost: x\r\n\r\n";
    let m = parse(raw, Profile::Lenient);
    let d = has(&m, DiagnosticCode::LeadingWhitespaceBeforeStartLine).unwrap();
    assert_eq!(d.severity(), Severity::Info);
    assert_eq!(text(raw, d), b"\r\n \t");
    let StartLine::Request { method, .. } = m.head().start() else {
        panic!()
    };
    assert_eq!(&raw[method.range()], b"GET");
    assert_eq!(
        count(&m, DiagnosticCode::LeadingWhitespaceBeforeStartLine),
        1
    );
}

#[test]
fn non_canonical_header_case() {
    let raw = b"GET / HTTP/1.1\r\nhost: x\r\nX-a: 1\r\nContent-Length: 0\r\n\r\n";
    let m = parse(raw, Profile::Lenient);
    assert_eq!(count(&m, DiagnosticCode::NonCanonicalHeaderCase), 2);
    let d = has(&m, DiagnosticCode::NonCanonicalHeaderCase).unwrap();
    assert_eq!(text(raw, d), b"host");
    assert_eq!(d.severity(), Severity::Info);
    assert!(has(&m, DiagnosticCode::MissingHost).is_none());
}

#[test]
fn invalid_chars_in_name_and_value() {
    let raw = b"GET / HTTP/1.1\r\nHo\x7fst: x\x01\r\nX-B: \xff\xfe\r\n\r\n";
    let m = parse(raw, Profile::Lenient);
    let d = has(&m, DiagnosticCode::InvalidTokenCharInName).unwrap();
    assert_eq!(text(raw, d), b"Ho\x7fst");
    let d = has(&m, DiagnosticCode::InvalidCharInValue).unwrap();
    assert_eq!(text(raw, d), b"\x01");
    assert_eq!(count(&m, DiagnosticCode::InvalidCharInValue), 1); // high bytes are fine
}

#[test]
fn colonless_line_is_an_invalid_name() {
    let raw = b"GET / HTTP/1.1\r\nHost: x\r\nnocolon\r\n\r\n";
    let m = parse(raw, Profile::Lenient);
    assert_eq!(m.head().headers().len(), 2);
    assert!(m.head().headers()[1].value().is_empty());
    let d = has(&m, DiagnosticCode::InvalidTokenCharInName).unwrap();
    assert_eq!(text(raw, d), b"nocolon");
}

#[test]
fn missing_and_duplicate_host() {
    let m = parse(b"GET / HTTP/1.1\r\n\r\n", Profile::Lenient);
    assert!(has(&m, DiagnosticCode::MissingHost).is_some());
    let m = parse(
        b"GET / HTTP/1.1\r\nHost: a\r\nhost: b\r\n\r\n",
        Profile::Lenient,
    );
    assert!(has(&m, DiagnosticCode::DuplicateHost).is_some());
    assert!(has(&m, DiagnosticCode::MissingHost).is_none());
    let m = parse(b"GET / HTTP/1.1\r\nHost: a\r\n\r\n", Profile::Lenient);
    assert!(has(&m, DiagnosticCode::DuplicateHost).is_none());
}

#[test]
fn absolute_form_and_versions() {
    let raw = b"GET http://x/ HTTP/1.1\r\nHost: x\r\n\r\n";
    let m = parse(raw, Profile::Lenient);
    assert_eq!(
        text(raw, has(&m, DiagnosticCode::AbsoluteFormTarget).unwrap()),
        b"http://x/"
    );
    assert!(has(
        &parse(b"GET /\r\n\r\n", Profile::Lenient),
        DiagnosticCode::Http09Request
    )
    .is_some());
    for v in [&b"HTTP/1.7"[..], b"HTTP/2", b"http/1.1"] {
        let mut raw = b"GET / ".to_vec();
        raw.extend_from_slice(v);
        raw.extend_from_slice(b"\r\nHost: x\r\n\r\n");
        let m = parse(&raw, Profile::Lenient);
        assert_eq!(
            text(&raw, has(&m, DiagnosticCode::VersionUnusual).unwrap()),
            v
        );
    }
    let m = parse(b"GET / HTTP/1.0\r\nHost: x\r\n\r\n", Profile::Lenient);
    assert!(has(&m, DiagnosticCode::VersionUnusual).is_none());
}

#[test]
fn short_request_lines() {
    let raw = b"GET /\r\nHost: x\r\n\r\n";
    let m = parse(raw, Profile::Strict);
    let d = has(&m, DiagnosticCode::Http09Request).unwrap();
    assert_eq!(d.severity(), Severity::Error);
    let StartLine::Request {
        target, version, ..
    } = m.head().start()
    else {
        panic!()
    };
    assert_eq!(&raw[target.range()], b"/");
    assert!(version.is_empty());
    let m = parse(b"GET\r\nHost: x\r\n\r\n", Profile::Lenient);
    assert!(has(&m, DiagnosticCode::Http09Request).is_some());
    let StartLine::Request {
        target, version, ..
    } = m.head().start()
    else {
        panic!()
    };
    assert!(target.is_empty() && version.is_empty());
}

#[test]
fn expect_100_continue() {
    let raw = b"POST / HTTP/1.1\r\nHost: x\r\nExpect: 100-Continue\r\n\r\n";
    let m = parse(raw, Profile::Lenient);
    let d = has(&m, DiagnosticCode::Expect100Continue).unwrap();
    assert_eq!(d.severity(), Severity::Info);
    assert_eq!(text(raw, d), b"100-Continue");
    let m = parse(
        b"POST / HTTP/1.1\r\nHost: x\r\nExpect: other\r\n\r\n",
        Profile::Lenient,
    );
    assert!(has(&m, DiagnosticCode::Expect100Continue).is_none());
}

#[test]
fn lenient_never_errors_on_terminated_head() {
    for raw in [
        &b"\r\n\r\nGET / HTTP/1.1\r\nHost: x\r\n\r\n"[..],
        b"G\x01T / HTTP/1.1\r\nHo\x7fst: x\x01\r\n\r\n",
        b"GET / HTTP/1.1\r\nX-A: \xff\xfe\r\nHost: x\r\n\r\n",
        b"GET\n\n",
        b"x\n\r\n",
        b"\x00\r\n\r\n",
        b"GET / HTTP/1.1\r\n \r\n\r\n",
        b"GET / HTTP/1.1\r\n: v\r\n\r\n",
        b"GET / HTTP/1.1\r\n :\r\n\r\n",
    ] {
        assert!(
            Parser::request(&ParserConfig::default()).feed(raw).is_ok(),
            "{raw:?}"
        );
    }
}

#[test]
fn every_byte_sequence_is_handled_without_panic() {
    // Exhaustive over a small alphabet of the interesting bytes.
    let alphabet = [b'\r', b'\n', b' ', b'\t', b':', b'a', b'H', 0x01, 0xff];
    let mut buf = [0u8; 6];
    let total = alphabet.len().pow(6);
    for mut n in 0..total {
        for slot in &mut buf {
            *slot = alphabet[n % alphabet.len()];
            n /= alphabet.len();
        }
        let whole = Parser::request(&ParserConfig::default()).feed(&buf);
        // Byte-at-a-time feeding must agree with one-shot feeding.
        let mut p = Parser::request(&ParserConfig::default());
        let mut last = None;
        for b in &buf {
            match p.feed(core::slice::from_ref(b)) {
                Ok(s) => {
                    let done = matches!(s.event, Progress::Head(_));
                    last = Some(Ok(s.event));
                    if done {
                        break;
                    }
                }
                Err(e) => {
                    last = Some(Err(e));
                    break;
                }
            }
        }
        match (whole, last) {
            (Ok(a), Some(Ok(b))) => assert_eq!(
                matches!(a.event, Progress::Head(_)),
                matches!(b, Progress::Head(_)),
                "{buf:?}"
            ),
            (Err(a), Some(Err(b))) => assert_eq!(a, b, "{buf:?}"),
            (a, b) => panic!("mismatch {buf:?}: {a:?} vs {b:?}"),
        }
    }
}

#[test]
fn errors_poison_the_parser_until_reset() {
    let cfg = ParserConfig::builder()
        .limits(ParseLimits::builder().head_size(16).build())
        .build();
    let mut p = Parser::request(&cfg);
    assert!(matches!(
        p.feed(b"GET /aaaaaaaaaaaaaaaaaaaaaaaaa"),
        Err(ParseError::HeadTooLarge)
    ));
    assert!(matches!(p.feed(b"x"), Err(ParseError::HeadTooLarge)));
    assert!(matches!(p.feed_eof(), Err(ParseError::HeadTooLarge)));
    p.reset();
    assert!(matches!(
        p.feed(b"GET /\r\n\r\n").unwrap().event,
        Progress::Head(_)
    ));
    let mut p = Parser::request(&ParserConfig::default());
    assert!(matches!(p.feed_eof(), Err(ParseError::UnexpectedEof)));
    assert!(matches!(
        p.feed(b"GET / HTTP/1.1\r\n\r\n"),
        Err(ParseError::UnexpectedEof)
    ));
}

#[test]
fn spaced_content_length_is_found_by_lookup() {
    let m = parse(
        b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length : 5\r\n\r\n",
        Profile::Lenient,
    );
    assert!(has(&m, DiagnosticCode::WhitespaceBeforeColon).is_some());
    let v = m.head().header(m.bytes(), "Content-Length").unwrap();
    assert_eq!(&m.bytes()[v.range()], b"5");
}

#[test]
fn control_byte_in_folded_value() {
    let raw = b"GET / HTTP/1.1\r\nHost: x\r\nX-A: 1\r\n \x01\r\n\r\n";
    let m = parse(raw, Profile::Lenient);
    assert!(has(&m, DiagnosticCode::ObsFold).is_some());
    assert_eq!(
        text(raw, has(&m, DiagnosticCode::InvalidCharInValue).unwrap()),
        b"\x01"
    );
}
