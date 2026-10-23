//! The first request on a client connection: what kind it is and where it
//! goes.

use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};

use bytes::{Bytes, BytesMut};
use crosswire_client::Conn;
use crosswire_h1::{ParseError, Parser, Progress, RawMessage, StartLine};
use tokio::io::AsyncReadExt;

use crate::{ProxyError, ProxyLimits};

/// 400 body: the request target is not a valid `host:port`.
pub(crate) const BAD_AUTHORITY: &str =
    "Crosswire refused the request: the target is not a valid host and port.";
/// 400 body: the request could not be parsed.
pub(crate) const MALFORMED: &str = "Crosswire could not parse the request.";
/// 431 body: the request head was over the configured limits.
pub(crate) const HEAD_TOO_LARGE: &str = "Crosswire refused the request: its head is too large.";
/// 400 body: bytes arrived after a CONNECT head before the tunnel existed.
pub(crate) const EARLY_DATA: &str =
    "Crosswire refused the request: data arrived before the tunnel was established.";
/// 407 body.
pub(crate) const AUTH_REQUIRED: &str = "Crosswire requires proxy authentication.";

/// The local name that reaches the info page.
const LOCAL_NAME: &str = "crosswire";

/// What the first request on a connection asks for.
#[derive(Debug)]
pub(crate) enum FirstRequest {
    /// `CONNECT host:port`. `authority` is the host without IPv6 brackets.
    Connect {
        authority: String,
        port: u16,
        head: RawMessage,
    },
    /// An absolute-form request (`GET http://host/...`). `host` is without
    /// IPv6 brackets; `leftover` is what followed the head in the last read
    /// (the start of the body or of a pipelined request).
    Absolute {
        host: String,
        port: u16,
        tls: bool,
        head: RawMessage,
        leftover: Bytes,
    },
    /// A request for the info page (origin-form with a local `Host`, or
    /// absolute-form `http://crosswire/...`).
    Local { head: RawMessage },
    /// Answered with a generated `status` whose body is `reason`, then
    /// closed.
    Refused { status: u16, reason: &'static str },
}

/// Reads and classifies the first request head on `io`, within
/// `limits.header_read`. `local` is the listener's own address, which
/// reaches the info page.
///
/// Returns the classification and the bytes consumed for the head (for a
/// refusal, everything read).
///
/// # Errors
/// [`ProxyError::Timeout`] when the head does not arrive in time,
/// [`ProxyError::Io`] on a read error, [`ProxyError::Protocol`] when the
/// client closes before sending anything.
pub(crate) async fn read_first_request(
    io: &mut dyn Conn,
    limits: &ProxyLimits,
    local: SocketAddr,
) -> Result<(FirstRequest, Bytes), ProxyError> {
    let read = async {
        let mut parser = Parser::request(&limits.parse);
        let mut seen = BytesMut::new();
        let mut buf = vec![0u8; 8 * 1024];
        loop {
            let n = io.read(&mut buf).await.map_err(ProxyError::Io)?;
            if n == 0 {
                if seen.is_empty() {
                    return Err(ProxyError::Protocol(
                        "client closed before sending a request".into(),
                    ));
                }
                return Ok((refused(400, MALFORMED), seen.freeze()));
            }
            let chunk = &buf[..n];
            match parser.feed(chunk) {
                Ok(step) => match step.event {
                    Progress::NeedMore => seen.extend_from_slice(chunk),
                    Progress::Head(head) => {
                        seen.extend_from_slice(&chunk[..step.consumed]);
                        let leftover = Bytes::copy_from_slice(&chunk[step.consumed..]);
                        return Ok((classify(head, leftover, local), seen.freeze()));
                    }
                    _ => return Ok((refused(400, MALFORMED), seen.freeze())),
                },
                Err(e) => {
                    seen.extend_from_slice(chunk);
                    let r = match e {
                        ParseError::HeadTooLarge
                        | ParseError::TooManyHeaders
                        | ParseError::LineTooLong => refused(431, HEAD_TOO_LARGE),
                        _ => refused(400, MALFORMED),
                    };
                    return Ok((r, seen.freeze()));
                }
            }
        }
    };
    tokio::time::timeout(limits.header_read, read)
        .await
        .map_err(|_| ProxyError::Timeout("request head"))?
}

fn refused(status: u16, reason: &'static str) -> FirstRequest {
    FirstRequest::Refused { status, reason }
}

fn classify(head: RawMessage, leftover: Bytes, local: SocketAddr) -> FirstRequest {
    let buf = head.bytes();
    let StartLine::Request {
        method,
        target,
        version,
    } = *head.head().start()
    else {
        return refused(400, MALFORMED);
    };
    let version = &buf[version.range()];
    if version != b"HTTP/1.1" && version != b"HTTP/1.0" {
        return refused(400, MALFORMED);
    }
    let Ok(target) = std::str::from_utf8(&buf[target.range()]) else {
        return refused(400, BAD_AUTHORITY);
    };
    let target = target.to_string();

    if &buf[method.range()] == b"CONNECT" {
        let Some((host, Some(port))) = split_authority(&target) else {
            return refused(400, BAD_AUTHORITY);
        };
        if !valid_authority(host) {
            return refused(400, BAD_AUTHORITY);
        }
        if !leftover.is_empty() {
            return refused(400, EARLY_DATA);
        }
        return FirstRequest::Connect {
            authority: unbracket(host).to_string(),
            port,
            head,
        };
    }

    if target.starts_with('/') {
        let host = head
            .head()
            .header(buf, "host")
            .and_then(|s| std::str::from_utf8(&buf[s.range()]).ok());
        return match host {
            Some(h) if is_local_host(h, local) => FirstRequest::Local { head },
            _ => refused(400, crate::generated::NOT_A_PROXY_REQUEST),
        };
    }

    let Some((tls, rest)) = strip_scheme(&target) else {
        return refused(400, crate::generated::NOT_A_PROXY_REQUEST);
    };
    let authority = &rest[..rest.find(['/', '?', '#']).unwrap_or(rest.len())];
    let Some((host, port)) = split_authority(authority) else {
        return refused(400, BAD_AUTHORITY);
    };
    if !valid_authority(host) {
        return refused(400, BAD_AUTHORITY);
    }
    if !tls && is_local_host(authority, local) {
        return FirstRequest::Local { head };
    }
    FirstRequest::Absolute {
        host: unbracket(host).to_string(),
        port: port.unwrap_or(if tls { 443 } else { 80 }),
        tls,
        head,
        leftover,
    }
}

/// The destination of an absolute-form target (`http[s]://authority/...`):
/// whether it is TLS, the authority as written, the host without IPv6
/// brackets, and the port (the scheme's default when absent). `None` when
/// the target is not absolute-form or its authority is invalid.
pub(crate) fn absolute_parts(target: &str) -> Option<AbsoluteTarget> {
    let (tls, rest) = strip_scheme(target)?;
    let authority = &rest[..rest.find(['/', '?', '#']).unwrap_or(rest.len())];
    let (host, port) = split_authority(authority)?;
    if !valid_authority(host) {
        return None;
    }
    Some(AbsoluteTarget {
        tls,
        authority: authority.to_string(),
        host: unbracket(host).to_string(),
        port: port.unwrap_or(if tls { 443 } else { 80 }),
    })
}

/// See [`absolute_parts`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AbsoluteTarget {
    pub(crate) tls: bool,
    pub(crate) authority: String,
    pub(crate) host: String,
    pub(crate) port: u16,
}

/// `http://` or `https://` (any case) stripped, with whether it was TLS.
fn strip_scheme(target: &str) -> Option<(bool, &str)> {
    let lower = |n: usize| target.get(..n).map(str::to_ascii_lowercase);
    if lower(7).as_deref() == Some("http://") {
        Some((false, &target[7..]))
    } else if lower(8).as_deref() == Some("https://") {
        Some((true, &target[8..]))
    } else {
        None
    }
}

/// Splits `host[:port]` (host possibly a bracketed IPv6 literal). A port,
/// when present, must be 1 to 5 digits and nonzero.
fn split_authority(a: &str) -> Option<(&str, Option<u16>)> {
    let (host, port) = if a.starts_with('[') {
        let end = a.find(']')?;
        let rest = &a[end + 1..];
        let port = if rest.is_empty() {
            None
        } else {
            Some(rest.strip_prefix(':')?)
        };
        (&a[..=end], port)
    } else {
        match a.rsplit_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (a, None),
        }
    };
    let port = match port {
        None => None,
        Some(p) => {
            if p.is_empty() || p.len() > 5 || !p.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            match p.parse::<u16>() {
                Ok(0) | Err(_) => return None,
                Ok(n) => Some(n),
            }
        }
    };
    Some((host, port))
}

fn unbracket(host: &str) -> &str {
    host.strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host)
}

/// Whether `host` is acceptable as a request target host: a name of at most
/// 253 bytes whose labels are 1 to 63 of `[A-Za-z0-9-]` not starting or
/// ending with `-`, an IPv4 literal, or a bracketed IPv6 literal.
pub(crate) fn valid_authority(host: &str) -> bool {
    if let Some(inner) = host.strip_prefix('[') {
        return inner
            .strip_suffix(']')
            .is_some_and(|v6| v6.parse::<Ipv6Addr>().is_ok());
    }
    if host.parse::<Ipv4Addr>().is_ok() {
        return true;
    }
    !host.is_empty()
        && host.len() <= 253
        && host.split('.').all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

/// Whether a `Host` value (or absolute-form authority) names this proxy:
/// `crosswire`, `crosswire:PORT`, the listener's own `host:port`, or
/// `localhost:PORT` when the listener is on loopback.
fn is_local_host(host: &str, local: SocketAddr) -> bool {
    let host = host.trim().to_ascii_lowercase();
    if host == LOCAL_NAME {
        return true;
    }
    if let Some(port) = host
        .strip_prefix(LOCAL_NAME)
        .and_then(|r| r.strip_prefix(':'))
    {
        return !port.is_empty() && port.len() <= 5 && port.bytes().all(|b| b.is_ascii_digit());
    }
    host == local.to_string()
        || (local.ip().is_loopback() && host == format!("localhost:{}", local.port()))
}

/// The path (without query or fragment) of an origin-form or absolute-form
/// request target; `/` when it has none.
pub(crate) fn request_path(head: &RawMessage) -> String {
    let StartLine::Request { target, .. } = *head.head().start() else {
        return "/".into();
    };
    let target = String::from_utf8_lossy(&head.bytes()[target.range()]).into_owned();
    let rest = match strip_scheme(&target) {
        Some((_, rest)) => match rest.find('/') {
            Some(i) => &rest[i..],
            None => "",
        },
        None => &target[..],
    };
    let path = &rest[..rest.find(['?', '#']).unwrap_or(rest.len())];
    if path.is_empty() {
        "/".into()
    } else {
        path.to_string()
    }
}

/// The request method's bytes.
pub(crate) fn request_method(head: &RawMessage) -> &[u8] {
    match head.head().start() {
        StartLine::Request { method, .. } => &head.bytes()[method.range()],
        _ => b"",
    }
}

/// Constant-time equality for equal-length inputs (the length itself is not
/// secret).
pub(crate) fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Whether `head` carries `Proxy-Authorization: Basic <expected>`, where
/// `expected` is `base64(user:pass)`. The token is compared in constant
/// time.
pub(crate) fn proxy_auth_ok(head: &RawMessage, expected: &[u8]) -> bool {
    let buf = head.bytes();
    let Some(span) = head.head().header(buf, "proxy-authorization") else {
        return false;
    };
    let value = buf[span.range()].trim_ascii();
    let Some(sp) = value.iter().position(|&b| b == b' ' || b == b'\t') else {
        return false;
    };
    let (scheme, token) = value.split_at(sp);
    scheme.eq_ignore_ascii_case(b"basic") && ct_eq(token.trim_ascii(), expected)
}

/// The 407 response asking for proxy credentials.
pub(crate) fn proxy_auth_required() -> Bytes {
    Bytes::from(format!(
        "HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"crosswire\"\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nX-Crosswire-Generated: 1\r\n\r\n{AUTH_REQUIRED}",
        AUTH_REQUIRED.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crosswire_h1::ParserConfig;

    fn parse(req: &[u8]) -> RawMessage {
        let mut p = Parser::request(&ParserConfig::default());
        match p.feed(req).unwrap().event {
            Progress::Head(h) => h,
            other => panic!("{other:?}"),
        }
    }

    fn local() -> SocketAddr {
        "127.0.0.1:8080".parse().unwrap()
    }

    #[test]
    fn authority_rules() {
        assert!(valid_authority("example.com"));
        assert!(valid_authority("a-b.c0"));
        assert!(valid_authority("10.0.0.1"));
        assert!(valid_authority("[::1]"));
        assert!(!valid_authority("::1"));
        assert!(!valid_authority("[not-v6]"));
        assert!(!valid_authority(""));
        assert!(!valid_authority("-a.test"));
        assert!(!valid_authority("a-.test"));
        assert!(!valid_authority("a..test"));
        assert!(!valid_authority("bad host"));
        assert!(!valid_authority("a_b.test"));
        assert!(!valid_authority(&"a".repeat(64)));
        let n253 = format!("{0}.{0}.{0}.{1}", "a".repeat(63), "a".repeat(61));
        assert_eq!(n253.len(), 253);
        assert!(valid_authority(&n253));
        assert!(!valid_authority(&format!("{n253}a")));
    }

    #[test]
    fn authority_split() {
        assert_eq!(split_authority("a.test:443"), Some(("a.test", Some(443))));
        assert_eq!(split_authority("a.test"), Some(("a.test", None)));
        assert_eq!(split_authority("[::1]:8443"), Some(("[::1]", Some(8443))));
        assert_eq!(split_authority("[::1]"), Some(("[::1]", None)));
        assert_eq!(split_authority("a.test:0"), None);
        assert_eq!(split_authority("a.test:+1"), None);
        assert_eq!(split_authority("a.test:65536"), None);
        assert_eq!(split_authority("a.test:"), None);
        assert_eq!(split_authority("[::1]x"), None);
    }

    #[test]
    fn classifies_absolute_form() {
        let h = parse(b"GET http://u.test:8080/p?q HTTP/1.1\r\nHost: u.test\r\n\r\n");
        match classify(h, Bytes::from_static(b"next"), local()) {
            FirstRequest::Absolute {
                host,
                port,
                tls,
                leftover,
                ..
            } => {
                assert_eq!((host.as_str(), port, tls), ("u.test", 8080, false));
                assert_eq!(leftover.as_ref(), b"next");
            }
            other => panic!("{other:?}"),
        }
        let h = parse(b"GET HTTPS://[::1] HTTP/1.1\r\n\r\n");
        assert!(matches!(
            classify(h, Bytes::new(), local()),
            FirstRequest::Absolute { ref host, port: 443, tls: true, .. } if host == "::1"
        ));
        let h = parse(b"GET http://crosswire/ca.pem HTTP/1.1\r\n\r\n");
        let FirstRequest::Local { head } = classify(h, Bytes::new(), local()) else {
            panic!()
        };
        assert_eq!(request_path(&head), "/ca.pem");
        let h = parse(b"GET http://127.0.0.1:8080 HTTP/1.1\r\n\r\n");
        assert!(matches!(
            classify(h, Bytes::new(), local()),
            FirstRequest::Local { .. }
        ));
    }

    #[test]
    fn connect_with_early_data_is_refused() {
        let h = parse(b"CONNECT a.test:443 HTTP/1.1\r\n\r\n");
        assert!(matches!(
            classify(h, Bytes::from_static(b"\x16\x03"), local()),
            FirstRequest::Refused {
                status: 400,
                reason: EARLY_DATA
            }
        ));
        let h = parse(b"CONNECT [::1]:443 HTTP/1.1\r\n\r\n");
        assert!(matches!(
            classify(h, Bytes::new(), local()),
            FirstRequest::Connect { ref authority, port: 443, .. } if authority == "::1"
        ));
        let h = parse(b"CONNECT a.test HTTP/1.1\r\n\r\n");
        assert!(matches!(
            classify(h, Bytes::new(), local()),
            FirstRequest::Refused { status: 400, .. }
        ));
    }

    #[test]
    fn local_hosts() {
        assert!(is_local_host("crosswire", local()));
        assert!(is_local_host("Crosswire:9", local()));
        assert!(is_local_host("127.0.0.1:8080", local()));
        assert!(is_local_host("localhost:8080", local()));
        assert!(!is_local_host("localhost:8081", local()));
        assert!(!is_local_host("crosswire.test", local()));
        assert!(!is_local_host("crosswire:", local()));
    }

    #[test]
    fn paths() {
        assert_eq!(request_path(&parse(b"GET /a?b HTTP/1.1\r\n\r\n")), "/a");
        assert_eq!(
            request_path(&parse(b"GET http://crosswire HTTP/1.1\r\n\r\n")),
            "/"
        );
    }

    #[test]
    fn ct_eq_compares() {
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"ab"));
        assert!(ct_eq(b"", b""));
    }

    #[test]
    fn proxy_auth_parsing() {
        let ok = parse(b"CONNECT a:1 HTTP/1.1\r\nProxy-Authorization: basic  dTpw \r\n\r\n");
        assert!(proxy_auth_ok(&ok, b"dTpw"));
        assert!(!proxy_auth_ok(&ok, b"dTpx"));
        let bearer = parse(b"CONNECT a:1 HTTP/1.1\r\nProxy-Authorization: Bearer dTpw\r\n\r\n");
        assert!(!proxy_auth_ok(&bearer, b"dTpw"));
        let none = parse(b"CONNECT a:1 HTTP/1.1\r\n\r\n");
        assert!(!proxy_auth_ok(&none, b"dTpw"));
    }
}
