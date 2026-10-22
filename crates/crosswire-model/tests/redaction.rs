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
