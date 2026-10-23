//! Responses the proxy writes itself.

// The listener and relay (later tasks) are the consumers.
#![allow(dead_code)]

use bytes::{Bytes, BytesMut};
use crosswire_h1::{Parser, ParserConfig, Progress, ResponseContext};
use crosswire_model::{Body, ConnectionInfo, Fidelity, Message};

/// Largest server-supplied substitution, in bytes.
const MAX_SUBSTITUTION: usize = 1024;

/// 502 body: the upstream connection failed.
pub const BAD_GATEWAY_CONNECT: &str = "Crosswire could not connect to {host}:{port}: {reason}";
/// 502 body: the upstream certificate was refused.
pub const BAD_GATEWAY_TLS: &str = "Crosswire refused the upstream certificate for {host}:{port}: {reason}. Pass --accept-invalid-cert {host} to override for this host (audited).";
/// 502 body: the destination gate refused the target.
pub const REFUSED_DESTINATION: &str = "Crosswire refused {host}:{port} ({refusal}). Allow it for this project with: crosswire serve --allow {host}:{port}";
/// 504 body: the upstream timed out.
pub const GATEWAY_TIMEOUT: &str = "Crosswire timed out waiting for {host}:{port} ({phase})";
/// 400 body: a direct (non-proxy) request.
pub const NOT_A_PROXY_REQUEST: &str = "Crosswire is a proxy. Configure your browser to use it, or open http://crosswire/ for the CA and setup guide.";

/// Truncates `s` to at most 1,024 bytes on a char boundary.
pub(crate) fn cap(s: &str) -> &str {
    if s.len() <= MAX_SUBSTITUTION {
        return s;
    }
    let mut end = MAX_SUBSTITUTION;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Replaces each `{key}` in `template` with its value from `pairs`. Values
/// are capped at 1,024 bytes and are not themselves rescanned.
pub(crate) fn fill(template: &str, pairs: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    'outer: while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let tail = &rest[open..];
        for (key, value) in pairs {
            let placeholder_len = key.len() + 2;
            if tail.len() >= placeholder_len
                && tail.as_bytes()[1..=key.len()] == *key.as_bytes()
                && tail.as_bytes()[key.len() + 1] == b'}'
            {
                out.push_str(cap(value));
                rest = &tail[placeholder_len..];
                continue 'outer;
            }
        }
        out.push('{');
        rest = &tail[1..];
    }
    out.push_str(rest);
    out
}

/// Builds a proxy-generated response and its recorded [`Message`].
///
/// Returns the wire bytes and a `Fidelity::Generated` message whose head
/// spans come from parsing those bytes.
///
/// # Panics
/// If `reason` contains CR or LF, or `status` is one that cannot carry a
/// body (1xx, 204, 304): the parser would not frame the body and the
/// recorded head would disagree with the bytes.
#[must_use]
pub(crate) fn generated(
    status: u16,
    reason: &str,
    body: &str,
    conn: ConnectionInfo,
) -> (Bytes, Message) {
    assert!(
        status >= 200 && status != 204 && status != 304,
        "status {status} cannot carry a generated body (1xx, 204, 304)"
    );
    assert!(
        !reason.contains(['\r', '\n']),
        "reason phrase must be a single line"
    );
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nX-Crosswire-Generated: 1\r\n\r\n",
        body.len()
    );
    let mut buf = BytesMut::with_capacity(head.len() + body.len());
    buf.extend_from_slice(head.as_bytes());
    buf.extend_from_slice(body.as_bytes());
    let bytes = buf.freeze();

    let mut parser = Parser::response(
        &ParserConfig::default(),
        ResponseContext::for_method(b"GET"),
    );
    let step = parser.feed(&bytes).expect("generated response parses");
    let Progress::Head(raw) = step.event else {
        panic!("generated response has a complete head");
    };
    let body_part = if body.is_empty() {
        Body::None
    } else {
        Body::inline(bytes.slice(head.len()..))
    };
    let msg = Message::http1_raw(&raw, body_part, conn).with_fidelity(Fidelity::Generated);
    (bytes, msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crosswire_model::Head;

    fn conn() -> ConnectionInfo {
        ConnectionInfo::new("a.test", 80)
    }

    #[test]
    fn cap_cuts_on_a_char_boundary() {
        // 2 bytes + 3-byte chars: byte 1024 falls inside a char.
        let s = format!("ab{}", "€".repeat(400));
        assert!(!s.is_char_boundary(MAX_SUBSTITUTION));
        let c = cap(&s);
        assert!(c.len() <= MAX_SUBSTITUTION);
        assert_eq!(c.len(), 2 + 3 * 340);
        assert!(c.is_char_boundary(c.len()));
        assert_eq!(c.chars().last(), Some('€'));
        let out = fill("x {reason} y", &[("reason", &s)]);
        assert!(out.starts_with("x ab€") && out.ends_with("€ y"));
        assert_eq!(out.len(), 2 + c.len() + 2);
    }

    #[test]
    fn cap_exact_and_under() {
        let exact = "a".repeat(1024);
        assert_eq!(cap(&exact), exact);
        let over = "a".repeat(1025);
        assert_eq!(cap(&over).len(), 1024);
        assert_eq!(cap("short"), "short");
    }

    #[test]
    fn fill_substitutes_known_keys_only() {
        assert_eq!(fill("a {x} {x} {z}", &[("x", "1")]), "a 1 1 {z}");
    }

    #[test]
    fn copy_strings_are_exact() {
        assert_eq!(
            BAD_GATEWAY_CONNECT,
            "Crosswire could not connect to {host}:{port}: {reason}"
        );
        assert_eq!(
            BAD_GATEWAY_TLS,
            "Crosswire refused the upstream certificate for {host}:{port}: {reason}. Pass --accept-invalid-cert {host} to override for this host (audited)."
        );
        assert_eq!(
            REFUSED_DESTINATION,
            "Crosswire refused {host}:{port} ({refusal}). Allow it for this project with: crosswire serve --allow {host}:{port}"
        );
        assert_eq!(
            GATEWAY_TIMEOUT,
            "Crosswire timed out waiting for {host}:{port} ({phase})"
        );
        assert_eq!(
            NOT_A_PROXY_REQUEST,
            "Crosswire is a proxy. Configure your browser to use it, or open http://crosswire/ for the CA and setup guide."
        );
    }

    #[test]
    fn generated_502_is_byte_exact_and_marked() {
        let body = "Crosswire could not connect to a.test:80: refused";
        let (bytes, msg) = generated(502, "Bad Gateway", body, conn());
        let expected = format!(
            "HTTP/1.1 502 Bad Gateway\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nX-Crosswire-Generated: 1\r\n\r\n{body}",
            body.len()
        );
        assert_eq!(bytes.as_ref(), expected.as_bytes());
        assert_eq!(msg.fidelity(), Fidelity::Generated);
        let Head::Http1Raw { bytes: head, .. } = msg.head() else {
            panic!("expected raw head");
        };
        assert_eq!(head.len(), expected.len() - body.len());
        assert_eq!(head.as_ref(), &bytes[..head.len()]);
    }

    #[test]
    fn empty_body_has_zero_length_and_real_spans() {
        let (bytes, msg) = generated(502, "Bad Gateway", "", conn());
        assert!(std::str::from_utf8(&bytes)
            .unwrap()
            .contains("Content-Length: 0\r\n"));
        let Head::Http1Raw { bytes: head, .. } = msg.head() else {
            panic!("expected raw head");
        };
        assert_eq!(head.as_ref(), bytes.as_ref());
        let mut p = Parser::response(
            &ParserConfig::default(),
            ResponseContext::for_method(b"GET"),
        );
        let Progress::Head(raw) = p.feed(&bytes).unwrap().event else {
            panic!()
        };
        let code = raw.head().status_code(raw.bytes());
        assert_eq!(code, Some(502));
        assert!(raw.head().header(raw.bytes(), "Content-Length").is_some());
    }

    #[test]
    #[should_panic(expected = "cannot carry a generated body")]
    fn bodyless_status_is_rejected() {
        let _ = generated(204, "No Content", "x", conn());
    }
}
