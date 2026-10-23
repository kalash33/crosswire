//! The request-head edits the proxy makes before forwarding, each recorded
//! as a [`Diff`] against the head bytes as the client sent them.

use bytes::{Bytes, BytesMut};
use crosswire_core::{ChangeOp, ChangeReason, Span};
use crosswire_h1::{RawMessage, StartLine};
use crosswire_model::Diff;

/// Hop-by-hop headers the proxy removes. `Connection` is relayed as-is.
const STRIPPED: [&str; 2] = ["proxy-connection", "proxy-authorization"];

/// How the request reached the proxy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    /// Inside a CONNECT tunnel: the request is already origin-form.
    Tunnel,
    /// Absolute-form to the proxy. `host` is the target's authority
    /// (`host[:port]`), used for a `Host` line when the request has none.
    Plain { host: String },
}

/// One edit: replace `span` of the original with `after`.
struct Edit {
    op: ChangeOp,
    span: core::ops::Range<usize>,
    after: Bytes,
    reason: ChangeReason,
}

/// Rewrites a request head for the upstream.
///
/// Both modes delete the `Proxy-Connection` and `Proxy-Authorization` lines
/// (`Delete`, `HopByHop`). `Plain` also replaces an absolute-form target
/// with its origin form (`Replace`, `Custom("absolute-form")`) and, when the
/// request has no `Host`, inserts `Host: {host}\r\n` after the start line
/// (`Insert`, `Custom("host-from-target")`). Diff spans index the original
/// head bytes; every other byte is copied unchanged.
pub(crate) fn rewrite_request_head(head: &RawMessage, mode: &Mode) -> (Bytes, Vec<Diff>) {
    let buf = head.bytes();
    let view = head.head();
    let mut edits = Vec::new();

    if let Mode::Plain { host } = mode {
        if let StartLine::Request { target, .. } = view.start() {
            if let Some(origin) = origin_form(&buf[target.range()]) {
                edits.push(Edit {
                    op: ChangeOp::Replace,
                    span: target.range(),
                    after: Bytes::from(origin),
                    reason: ChangeReason::Custom("absolute-form".into()),
                });
            }
        }
        if view.header(buf, "host").is_none() {
            let at = buf
                .iter()
                .position(|&b| b == b'\n')
                .map_or(buf.len(), |i| i + 1);
            edits.push(Edit {
                op: ChangeOp::Insert,
                span: at..at,
                after: Bytes::from(format!("Host: {host}\r\n")),
                reason: ChangeReason::Custom("host-from-target".into()),
            });
        }
    }

    for h in view.headers() {
        let name = trim_end(&buf[h.name().range()]);
        if STRIPPED
            .iter()
            .any(|s| name.eq_ignore_ascii_case(s.as_bytes()))
        {
            edits.push(Edit {
                op: ChangeOp::Delete,
                span: h.line().range(),
                after: Bytes::new(),
                reason: ChangeReason::HopByHop,
            });
        }
    }

    if edits.is_empty() {
        return (buf.clone(), Vec::new());
    }
    edits.sort_by_key(|e| (e.span.start, e.span.end));
    let mut out = BytesMut::with_capacity(buf.len() + 64);
    let mut at = 0;
    let mut diffs = Vec::with_capacity(edits.len());
    for e in edits {
        out.extend_from_slice(&buf[at..e.span.start]);
        out.extend_from_slice(&e.after);
        at = e.span.end;
        diffs.push(Diff::new(
            e.op,
            span(&e.span),
            buf.slice(e.span.clone()),
            e.after,
            e.reason,
        ));
    }
    out.extend_from_slice(&buf[at..]);
    (out.freeze(), diffs)
}

fn span(r: &core::ops::Range<usize>) -> Span {
    let c = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
    Span::new(c(r.start), c(r.end))
}

fn trim_end(b: &[u8]) -> &[u8] {
    let keep = b.iter().rposition(|&c| c != b' ' && c != b'\t');
    &b[..keep.map_or(0, |i| i + 1)]
}

/// The origin form of an `http://` or `https://` target (any case): the
/// path and query after the authority, `/` when empty. `None` for any
/// other target.
pub(crate) fn origin_form(target: &[u8]) -> Option<Vec<u8>> {
    let rest = strip_scheme(target)?;
    let start = rest
        .iter()
        .position(|&b| matches!(b, b'/' | b'?' | b'#'))
        .unwrap_or(rest.len());
    let tail = &rest[start..];
    let mut out = Vec::with_capacity(tail.len() + 1);
    if !tail.starts_with(b"/") {
        out.push(b'/');
    }
    out.extend_from_slice(tail);
    Some(out)
}

fn strip_scheme(target: &[u8]) -> Option<&[u8]> {
    for scheme in [&b"http://"[..], &b"https://"[..]] {
        if target.len() >= scheme.len() && target[..scheme.len()].eq_ignore_ascii_case(scheme) {
            return Some(&target[scheme.len()..]);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crosswire_h1::{Parser, ParserConfig, Progress};

    fn parse(bytes: &[u8]) -> RawMessage {
        let mut p = Parser::request(&ParserConfig::default());
        match p.feed(bytes).unwrap().event {
            Progress::Head(m) => m,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn origin_forms() {
        assert_eq!(origin_form(b"http://a.test/p?q").unwrap(), b"/p?q");
        assert_eq!(origin_form(b"HTTPS://a.test:8443").unwrap(), b"/");
        assert_eq!(origin_form(b"http://a.test?x=1").unwrap(), b"/?x=1");
        assert_eq!(origin_form(b"/p"), None);
    }

    #[test]
    fn plain_rewrite_inserts_host_and_strips_proxy_headers() {
        let orig = b"GET http://a.test:81/x HTTP/1.1\r\nProxy-Authorization: Basic eA==\r\nAccept: */*\r\nproxy-connection : close\r\n\r\n";
        let (out, diffs) = rewrite_request_head(
            &parse(orig),
            &Mode::Plain {
                host: "a.test:81".into(),
            },
        );
        assert_eq!(
            &out[..],
            b"GET /x HTTP/1.1\r\nHost: a.test:81\r\nAccept: */*\r\n\r\n"
        );
        assert_eq!(diffs.len(), 4);
        for d in &diffs {
            assert_eq!(&orig[d.span().range()], &d.before()[..]);
        }
        let insert = diffs.iter().find(|d| d.op() == ChangeOp::Insert).unwrap();
        assert!(insert.span().is_empty());
        assert_eq!(
            *insert.reason(),
            ChangeReason::Custom("host-from-target".into())
        );
    }

    #[test]
    fn tunnel_rewrite_without_proxy_headers_is_identity() {
        let orig = b"GET /x HTTP/1.1\r\nHost: a\r\nConnection: close\r\n\r\n";
        let (out, diffs) = rewrite_request_head(&parse(orig), &Mode::Tunnel);
        assert_eq!(&out[..], &orig[..]);
        assert!(diffs.is_empty());
    }
}
