//! Redaction tests.
mod common;
use common::*;

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn default_policy_redacts_cookie_and_bearer() {
    let raw =
        parse_req(b"GET / HTTP/1.1\r\nHost: x\r\nCookie: a=1\r\nX-T: Bearer abc.def.ghi\r\n\r\n");
    let m = Message::http1_raw(&raw, Body::None, conn("x", 80));
    let r = RedactionPolicy::default_policy().redact_message(&m);
    let Head::Http1Raw { bytes, .. } = r.head() else {
        panic!()
    };
    assert!(!bytes.windows(3).any(|w| w == b"a=1"));
    assert!(!bytes.windows(7).any(|w| w == b"abc.def"));
    assert_eq!(r.fidelity(), Fidelity::Normalized);
}

#[test]
fn spans_are_fresh_and_head_still_parses() {
    let raw = parse_req(b"GET /p HTTP/1.1\r\nhost: x\r\nCOOKIE: a=1; b=2\r\nX-N: keep\r\n\r\n");
    let m = Message::http1_raw(&raw, Body::None, conn("x", 80));
    let r = RedactionPolicy::default_policy().redact_message(&m);
    let Head::Http1Raw { bytes, spans, .. } = r.head() else {
        panic!()
    };
    assert_eq!(
        bytes.as_ref(),
        b"GET /p HTTP/1.1\r\nhost: x\r\nCOOKIE: [REDACTED]\r\nX-N: keep\r\n\r\n"
    );
    let hs = spans.headers();
    assert_eq!(hs.len(), 3);
    assert_eq!(&bytes[hs[1].value().range()], b"[REDACTED]");
    assert_eq!(&bytes[hs[2].value().range()], b"keep");
    // The original is untouched.
    let Head::Http1Raw { bytes: ob, .. } = m.head() else {
        panic!()
    };
    assert!(contains(ob, b"a=1; b=2"));
    assert_eq!(m.fidelity(), Fidelity::Raw);
}

#[test]
fn response_set_cookie_and_value_patterns() {
    let raw = parse_resp(
        b"HTTP/1.1 200 OK\r\nSet-Cookie: s=1\r\nLocation: /cb?access_token=SECRET&x=1\r\nContent-Length: 0\r\n\r\n",
    );
    let m = Message::http1_raw(&raw, Body::None, conn("x", 80));
    let r = RedactionPolicy::default_policy().redact_message(&m);
    let Head::Http1Raw { bytes, .. } = r.head() else {
        panic!()
    };
    assert!(!contains(bytes, b"s=1"));
    assert!(!contains(bytes, b"SECRET"));
    assert!(contains(bytes, b"&x=1"));
    assert_eq!(r.framing(), m.framing());
}

#[test]
fn jwt_shape_is_redacted() {
    let raw = parse_req(
        b"GET / HTTP/1.1\r\nHost: x\r\nX-Y: tok eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.sig_-1 end\r\n\r\n",
    );
    let m = Message::http1_raw(&raw, Body::None, conn("x", 80));
    let r = RedactionPolicy::default_policy().redact_message(&m);
    let Head::Http1Raw { bytes, .. } = r.head() else {
        panic!()
    };
    assert!(!contains(bytes, b"eyJ"));
    assert!(contains(bytes, b"tok [REDACTED] end"));
}

#[test]
fn blob_body_untouched_and_tunnel_unchanged() {
    let raw = parse_req(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n");
    let blob = Body::Blob {
        hash: [1; 32],
        len: 5,
        truncated_at: None,
        capture_dropped: false,
        relay: None,
    };
    let m = Message::http1_raw(&raw, blob.clone(), conn("x", 80));
    let r = RedactionPolicy::default_policy().redact_message(&m);
    assert_eq!(r.body(), &blob);
}

#[test]
fn custom_policy_names_are_case_insensitive() {
    let raw = parse_req(b"GET / HTTP/1.1\r\nHost: x\r\nX-Secret : v\r\n\r\n");
    let m = Message::http1_raw(&raw, Body::None, conn("x", 80));
    let p = RedactionPolicy::new(vec!["x-SECRET".into()], vec![]);
    let r = p.redact_message(&m);
    let Head::Http1Raw { bytes, .. } = r.head() else {
        panic!()
    };
    assert!(contains(bytes, b"X-Secret : [REDACTED]\r\n"));
}

fn head_bytes(m: &Message) -> Vec<u8> {
    let Head::Http1Raw { bytes, .. } = m.head() else {
        panic!()
    };
    bytes.to_vec()
}

#[test]
fn request_target_access_token_is_redacted() {
    let raw = parse_req(b"GET /x?access_token=SECRET&y=2 HTTP/1.1\r\nHost: x\r\n\r\n");
    let m = Message::http1_raw(&raw, Body::None, conn("x", 80));
    let r = RedactionPolicy::default_policy().redact_message(&m);
    let b = head_bytes(&r);
    assert!(!contains(&b, b"SECRET"));
    assert!(b.starts_with(b"GET /x?[REDACTED]&y=2 HTTP/1.1\r\n"));
    let Head::Http1Raw { spans, .. } = r.head() else {
        panic!()
    };
    let StartLineSpans::Request {
        target, version, ..
    } = *spans.start_line()
    else {
        panic!()
    };
    assert_eq!(&b[target.range()], b"/x?[REDACTED]&y=2");
    assert_eq!(&b[version.range()], b"HTTP/1.1");
}

#[test]
fn request_target_jwt_is_redacted() {
    let raw =
        parse_req(b"GET /cb/eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.sig HTTP/1.1\r\nHost: x\r\n\r\n");
    let m = Message::http1_raw(&raw, Body::None, conn("x", 80));
    let r = RedactionPolicy::default_policy().redact_message(&m);
    let b = head_bytes(&r);
    assert!(!contains(&b, b"eyJ"));
    assert!(b.starts_with(b"GET /cb/[REDACTED] HTTP/1.1\r\n"));
}

#[test]
fn request_target_with_two_matches() {
    let raw = parse_req(b"GET /x?access_token=A1&mid=1&api_key=B2 HTTP/1.1\r\nHost: x\r\n\r\n");
    let m = Message::http1_raw(&raw, Body::None, conn("x", 80));
    let r = RedactionPolicy::default_policy().redact_message(&m);
    let b = head_bytes(&r);
    assert!(b.starts_with(b"GET /x?[REDACTED]&mid=1&[REDACTED] HTTP/1.1\r\n"));
    assert!(!contains(&b, b"A1") && !contains(&b, b"B2"));
}

#[test]
fn diffs_are_redacted() {
    use crosswire_core::{ChangeOp, ChangeReason, Span};
    let raw = parse_req(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n");
    let d = Diff::new(
        ChangeOp::Replace,
        Span::new(0, 4),
        bytes::Bytes::from_static(b"Host: x\r\nCookie: a=1\r\n"),
        bytes::Bytes::from_static(b"?access_token=TOK&k=1"),
        ChangeReason::InterceptEdit,
    );
    let m = Message::http1_raw(&raw, Body::None, conn("x", 80)).with_diffs(vec![d]);
    let r = RedactionPolicy::default_policy().redact_message(&m);
    let d = &r.diffs()[0];
    assert_eq!(d.before().as_ref(), b"Host: x\r\nCookie: [REDACTED]\r\n");
    assert_eq!(d.after().as_ref(), b"?[REDACTED]&k=1");
    assert_eq!(d.span(), Span::new(0, 4));
}

#[test]
fn overlapping_and_out_of_order_spans_do_not_panic() {
    use crosswire_core::Span;
    let bytes = bytes::Bytes::from_static(b"GET / HTTP/1.1\r\nCookie: abcdef\r\n\r\n");
    let sp = Span::new;
    let spans = HeadSpans::new(
        StartLineSpans::Request {
            method: sp(0, 3),
            target: sp(4, 5),
            version: sp(6, 14),
        },
        vec![
            HeaderSpanRecord::new(sp(16, 22), sp(26, 29), sp(16, 32)),
            HeaderSpanRecord::new(sp(16, 22), sp(24, 30), sp(16, 32)),
            HeaderSpanRecord::new(sp(16, 22), sp(500, 600), sp(16, 32)),
        ],
        FramingRecord::None,
    );
    let m = Message::new(
        MessageKind::Http1Raw,
        Fidelity::Raw,
        Head::Http1Raw {
            bytes,
            spans,
            parser_version: 1,
        },
        FramingRecord::None,
        Body::None,
        conn("x", 80),
    );
    let r = RedactionPolicy::default_policy().redact_message(&m);
    assert_eq!(r.fidelity(), Fidelity::Normalized);
    let b = head_bytes(&r);
    assert!(!contains(&b, b"abcdef"));
    assert!(contains(&b, b"Cookie: [REDACTED]\r\n"));
}

#[test]
fn interim_heads_are_redacted_like_the_head() {
    let raw = parse_resp(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
    let m = Message::http1_raw(&raw, Body::None, conn("x", 80)).with_interim(vec![
        bytes::Bytes::from_static(b"HTTP/1.1 100 Continue\r\n\r\n"),
        bytes::Bytes::from_static(
            b"HTTP/1.1 103 Early Hints\r\nSet-Cookie: s=SECRET\r\nLink: </a?api_key=K1>\r\n\r\n",
        ),
    ]);
    let r = RedactionPolicy::default_policy().redact_message(&m);
    assert_eq!(r.interim().len(), 2);
    assert_eq!(&r.interim()[0][..], b"HTTP/1.1 100 Continue\r\n\r\n");
    let hints = &r.interim()[1][..];
    assert!(!contains(hints, b"SECRET") && !contains(hints, b"K1"));
    assert!(contains(hints, b"Set-Cookie: [REDACTED]\r\n"));
    assert!(contains(hints, b"Link: </a?[REDACTED]\r\n"));
}
