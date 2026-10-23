//! Responses the proxy writes itself.

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
// Used by the listener and relay in later tasks.
#[allow(dead_code)]
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
/// If `reason` contains CR or LF, or the bytes fail to parse as a response
/// (callers pass fixed status codes and reason phrases).
#[must_use]
pub fn generated(status: u16, reason: &str, body: &str, conn: ConnectionInfo) -> (Bytes, Message) {
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

    #[test]
    fn fill_substitutes_and_caps() {
        let long = "é".repeat(600);
        let s = fill("a {x} {x} {y} {z}", &[("x", "1"), ("y", &long)]);
        assert!(s.starts_with("a 1 1 "));
        assert!(s.ends_with(" {z}"));
        assert!(s.len() < 1100);
    }
}
