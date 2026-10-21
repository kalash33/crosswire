//! Body framing resolution from the head.
use crosswire_core::{Diagnostic, DiagnosticCode, Profile};
use crosswire_h1::*;

fn parse(b: &[u8], profile: Profile) -> RawMessage {
    let cfg = ParserConfig::builder().profile(profile).build();
    match Parser::request(&cfg).feed(b).unwrap().event {
        Progress::Head(m) => m,
        other => panic!("expected head, got {other:?}"),
    }
}

fn parse_response(b: &[u8], ctx: ResponseContext) -> RawMessage {
    match Parser::response(&ParserConfig::default(), ctx)
        .feed(b)
        .unwrap()
        .event
    {
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

fn framing_of(b: &[u8]) -> BodyFraming {
    parse(b, Profile::Lenient).head().framing().clone()
}

fn framing_of_response(b: &[u8], ctx: ResponseContext) -> BodyFraming {
    parse_response(b, ctx).head().framing().clone()
}

#[test]
fn content_length_single() {
    assert!(matches!(
        framing_of(b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 5\r\n\r\n"),
        BodyFraming::ContentLength(5)
    ));
}

#[test]
fn duplicate_equal_content_length_is_diagnostic_not_error() {
    let m = parse(
        b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 5\r\nContent-Length: 5\r\n\r\n",
        Profile::Lenient,
    );
    assert!(has(&m, DiagnosticCode::DuplicateContentLength).is_some());
    assert!(matches!(m.head().framing(), BodyFraming::ContentLength(5)));
}

#[test]
fn conflicting_cl_and_te_resolves_to_chunked_with_rule() {
    let m = parse(
        b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 5\r\nTransfer-Encoding: chunked\r\n\r\n",
        Profile::Lenient,
    );
    let BodyFraming::Ambiguous {
        resolved,
        candidates,
        rule,
    } = m.head().framing()
    else {
        panic!()
    };
    assert!(matches!(**resolved, BodyFraming::Chunked));
    assert_eq!(candidates.len(), 2);
    assert_eq!(*rule, ResolutionRule::TransferEncodingWins);
    assert!(has(&m, DiagnosticCode::ConflictingFraming).is_some());
}

#[test]
fn differing_content_lengths_resolve_to_first() {
    let m = parse(
        b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 5\r\nContent-Length: 6\r\n\r\n",
        Profile::Lenient,
    );
    let BodyFraming::Ambiguous {
        resolved,
        candidates,
        rule,
    } = m.head().framing()
    else {
        panic!()
    };
    assert!(matches!(**resolved, BodyFraming::ContentLength(5)));
    assert_eq!(*rule, ResolutionRule::FirstContentLength);
    assert_eq!(
        candidates,
        &[BodyFraming::ContentLength(5), BodyFraming::ContentLength(6)]
    );
    assert!(has(&m, DiagnosticCode::DuplicateContentLength).is_some());
    assert!(has(&m, DiagnosticCode::ConflictingFraming).is_none());
}

#[test]
fn non_numeric_and_leading_plus() {
    let m = parse(
        b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 5x\r\n\r\n",
        Profile::Lenient,
    );
    assert!(has(&m, DiagnosticCode::ContentLengthNonNumeric).is_some());
    assert!(matches!(m.head().framing(), BodyFraming::None));
    let m = parse(
        b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: +5\r\n\r\n",
        Profile::Lenient,
    );
    assert!(has(&m, DiagnosticCode::ContentLengthLeadingPlus).is_some());
    assert!(matches!(m.head().framing(), BodyFraming::ContentLength(5)));
}

#[test]
fn unknown_transfer_coding_is_until_close_for_responses() {
    let m = parse_response(
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: gzip\r\n\r\n",
        ResponseContext::for_method(b"GET"),
    );
    assert!(has(&m, DiagnosticCode::UnknownTransferCoding).is_some());
    assert!(matches!(m.head().framing(), BodyFraming::UntilClose));
}

#[test]
fn response_context_no_body() {
    for (raw, ctx) in [
        (
            &b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\n"[..],
            ResponseContext::for_method(b"HEAD"),
        ),
        (
            b"HTTP/1.1 204 No Content\r\nContent-Length: 10\r\n\r\n",
            ResponseContext::for_method(b"GET"),
        ),
        (
            b"HTTP/1.1 304 Not Modified\r\nContent-Length: 10\r\n\r\n",
            ResponseContext::for_method(b"GET"),
        ),
        (
            b"HTTP/1.1 100 Continue\r\nContent-Length: 10\r\n\r\n",
            ResponseContext::for_method(b"GET"),
        ),
    ] {
        let m = parse_response(raw, ctx);
        assert!(
            matches!(m.head().framing(), BodyFraming::Ambiguous { resolved, .. } if matches!(**resolved, BodyFraming::None))
        );
        assert!(has(&m, DiagnosticCode::BodyAfterNoBodyStatus).is_some());
    }
}

#[test]
fn connect_2xx_and_101_are_upgrade() {
    assert!(matches!(
        framing_of_response(
            b"HTTP/1.1 200 OK\r\n\r\n",
            ResponseContext {
                method: Method::Other,
                is_connect: true
            }
        ),
        BodyFraming::Upgrade
    ));
    assert!(matches!(
        framing_of_response(
            b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n\r\n",
            ResponseContext::for_method(b"GET")
        ),
        BodyFraming::Upgrade
    ));
}

#[test]
fn response_without_length_is_until_close() {
    assert!(matches!(
        framing_of_response(
            b"HTTP/1.1 200 OK\r\n\r\n",
            ResponseContext::for_method(b"GET")
        ),
        BodyFraming::UntilClose
    ));
}

// Rules not covered by the brief's tests.

#[test]
fn no_body_status_without_length_headers_is_plain_none() {
    let m = parse_response(
        b"HTTP/1.1 204 No Content\r\n\r\n",
        ResponseContext::for_method(b"GET"),
    );
    assert!(matches!(m.head().framing(), BodyFraming::None));
    assert!(has(&m, DiagnosticCode::BodyAfterNoBodyStatus).is_none());
    let m = parse_response(
        b"HTTP/1.1 200 OK\r\n\r\n",
        ResponseContext::for_method(b"HEAD"),
    );
    assert!(matches!(m.head().framing(), BodyFraming::None));
}

#[test]
fn head_and_no_body_rules_name_their_rule_and_candidate() {
    let m = parse_response(
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n",
        ResponseContext::for_method(b"HEAD"),
    );
    let BodyFraming::Ambiguous {
        candidates, rule, ..
    } = m.head().framing()
    else {
        panic!()
    };
    assert_eq!(*rule, ResolutionRule::HeadMethod);
    assert_eq!(candidates, &[BodyFraming::Chunked]);
    let m = parse_response(
        b"HTTP/1.1 304 Not Modified\r\nContent-Length: 3\r\n\r\n",
        ResponseContext::for_method(b"GET"),
    );
    let BodyFraming::Ambiguous {
        candidates, rule, ..
    } = m.head().framing()
    else {
        panic!()
    };
    assert_eq!(*rule, ResolutionRule::NoBodyStatus);
    assert_eq!(candidates, &[BodyFraming::ContentLength(3)]);
}

#[test]
fn chunked_final_coding_and_case() {
    assert!(matches!(
        framing_of(b"POST / HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: gzip, CHUNKED\r\n\r\n"),
        BodyFraming::Chunked
    ));
    assert!(matches!(
        framing_of(b"POST / HTTP/1.1\r\nHost: x\r\ntransfer-encoding:\t chunked \r\n\r\n"),
        BodyFraming::Chunked
    ));
}

#[test]
fn request_with_non_final_chunked_is_none_with_diagnostic() {
    let m = parse(
        b"POST / HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked, gzip\r\n\r\n",
        Profile::Lenient,
    );
    assert!(has(&m, DiagnosticCode::UnknownTransferCoding).is_some());
    assert!(matches!(m.head().framing(), BodyFraming::None));
}

#[test]
fn te_with_invalid_cl_still_reports_cl_diagnostics() {
    let m = parse(
        b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: abc\r\nTransfer-Encoding: chunked\r\n\r\n",
        Profile::Lenient,
    );
    assert!(has(&m, DiagnosticCode::ContentLengthNonNumeric).is_some());
    assert!(has(&m, DiagnosticCode::ConflictingFraming).is_some());
    let BodyFraming::Ambiguous {
        resolved,
        candidates,
        rule,
    } = m.head().framing()
    else {
        panic!()
    };
    assert!(matches!(**resolved, BodyFraming::Chunked));
    assert_eq!(candidates, &[BodyFraming::Chunked]);
    assert_eq!(*rule, ResolutionRule::TransferEncodingWins);
}

#[test]
fn comma_list_content_length_is_treated_as_duplicates() {
    let m = parse(
        b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 5, 5\r\n\r\n",
        Profile::Lenient,
    );
    assert_eq!(count(&m, DiagnosticCode::DuplicateContentLength), 1);
    assert!(matches!(m.head().framing(), BodyFraming::ContentLength(5)));
    let m = parse(
        b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 5, 6\r\n\r\n",
        Profile::Lenient,
    );
    assert!(matches!(
        m.head().framing(),
        BodyFraming::Ambiguous {
            rule: ResolutionRule::FirstContentLength,
            ..
        }
    ));
}

#[test]
fn overflow_and_empty_content_length_are_non_numeric() {
    for v in ["99999999999999999999999", "", "-1", "0x10", "5 5"] {
        let raw = format!("POST / HTTP/1.1\r\nHost: x\r\nContent-Length: {v}\r\n\r\n");
        let m = parse(raw.as_bytes(), Profile::Lenient);
        assert!(
            has(&m, DiagnosticCode::ContentLengthNonNumeric).is_some(),
            "{v:?}"
        );
        assert!(matches!(m.head().framing(), BodyFraming::None), "{v:?}");
    }
    let m = parse_response(
        b"HTTP/1.1 200 OK\r\nContent-Length: nope\r\n\r\n",
        ResponseContext::for_method(b"GET"),
    );
    assert!(matches!(m.head().framing(), BodyFraming::UntilClose));
    assert_eq!(
        framing_of(b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 18446744073709551615\r\n\r\n"),
        BodyFraming::ContentLength(u64::MAX)
    );
}

#[test]
fn valid_value_survives_an_invalid_duplicate() {
    let m = parse(
        b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: x\r\nContent-Length: 7\r\n\r\n",
        Profile::Lenient,
    );
    assert!(has(&m, DiagnosticCode::ContentLengthNonNumeric).is_some());
    assert!(has(&m, DiagnosticCode::DuplicateContentLength).is_some());
    assert!(matches!(m.head().framing(), BodyFraming::ContentLength(7)));
}

#[test]
fn request_without_length_is_none() {
    assert!(matches!(
        framing_of(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n"),
        BodyFraming::None
    ));
}
