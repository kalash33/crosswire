//! Parsed message heads and framing.

use alloc::boxed::Box;
use alloc::vec::Vec;

use bytes::Bytes;
use crosswire_core::{Diagnostic, Span};

/// The start line of a message, as spans into the head bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum StartLine {
    /// A request line.
    Request {
        /// The method token.
        method: Span,
        /// The request target.
        target: Span,
        /// The HTTP version text.
        version: Span,
    },
    /// A status line.
    Response {
        /// The HTTP version text.
        version: Span,
        /// The status code text.
        status: Span,
        /// The reason phrase (possibly empty).
        reason: Span,
    },
}

/// One header field, as spans into the head bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeaderSpan {
    name: Span,
    value: Span,
    line: Span,
}

impl HeaderSpan {
    #[cfg_attr(not(test), expect(dead_code))] // used by the parser (Tasks 3-7)
    pub(crate) fn new(name: Span, value: Span, line: Span) -> Self {
        HeaderSpan { name, value, line }
    }

    /// The field name.
    #[must_use]
    pub fn name(&self) -> Span {
        self.name
    }

    /// The field value.
    #[must_use]
    pub fn value(&self) -> Span {
        self.value
    }

    /// The whole line, including the terminator.
    #[must_use]
    pub fn line(&self) -> Span {
        self.line
    }
}

/// The rule used to settle on a body framing when candidates conflict.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ResolutionRule {
    /// `Transfer-Encoding` took precedence over `Content-Length`.
    TransferEncodingWins,
    /// The first of several `Content-Length` values was used.
    FirstContentLength,
    /// The status code forbids a body (1xx, 204, 304).
    NoBodyStatus,
    /// The response answers a `HEAD` request.
    HeadMethod,
    /// The response is a successful answer to `CONNECT`.
    ConnectSuccess,
}

/// How the body of a message is delimited.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum BodyFraming {
    /// No body.
    None,
    /// A fixed number of bytes.
    ContentLength(u64),
    /// Chunked transfer coding.
    Chunked,
    /// The body runs until the connection closes.
    UntilClose,
    /// The connection was upgraded; no HTTP body follows.
    Upgrade,
    /// Conflicting framing signals; one was chosen by a rule.
    Ambiguous {
        /// The framing that was chosen.
        resolved: Box<BodyFraming>,
        /// Every framing the message could be read as.
        candidates: Vec<BodyFraming>,
        /// The rule that picked `resolved`.
        rule: ResolutionRule,
    },
}

/// A structural view of a message head.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadView {
    start: StartLine,
    headers: Vec<HeaderSpan>,
    framing: BodyFraming,
}

impl HeadView {
    #[cfg_attr(not(test), expect(dead_code))] // used by the parser (Tasks 3-7)
    pub(crate) fn new(start: StartLine, headers: Vec<HeaderSpan>, framing: BodyFraming) -> Self {
        HeadView {
            start,
            headers,
            framing,
        }
    }

    /// The start line.
    #[must_use]
    pub fn start(&self) -> &StartLine {
        &self.start
    }

    /// The header fields in wire order.
    #[must_use]
    pub fn headers(&self) -> &[HeaderSpan] {
        &self.headers
    }

    /// The determined body framing.
    #[must_use]
    pub fn framing(&self) -> &BodyFraming {
        &self.framing
    }

    /// Returns the value span of the first header whose name matches `name`
    /// ASCII-case-insensitively. `buf` must be the head bytes the spans index.
    /// An empty `name` never matches and returns `None`.
    #[must_use]
    pub fn header(&self, buf: &[u8], name: &str) -> Option<Span> {
        if name.is_empty() {
            return None;
        }
        self.headers
            .iter()
            .find(|h| {
                buf.get(h.name.range())
                    .is_some_and(|n| n.eq_ignore_ascii_case(name.as_bytes()))
            })
            .map(HeaderSpan::value)
    }
}

/// Whether a message is a request or a response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Kind {
    /// A request.
    Request,
    /// A response.
    Response,
}

/// A parsed message: frozen head bytes, a view over them, and diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawMessage {
    head: Bytes,
    view: HeadView,
    kind: Kind,
    body: Option<Bytes>,
    diagnostics: Vec<Diagnostic>,
}

impl RawMessage {
    #[cfg_attr(not(test), expect(dead_code))] // used by the parser (Tasks 3-7)
    pub(crate) fn new(
        head: Bytes,
        view: HeadView,
        kind: Kind,
        diagnostics: Vec<Diagnostic>,
    ) -> Self {
        RawMessage {
            head,
            view,
            kind,
            body: None,
            diagnostics,
        }
    }

    /// Attaches an in-memory body to the message.
    #[must_use]
    pub fn with_inline_body(mut self, body: Bytes) -> Self {
        self.body = Some(body);
        self
    }

    /// The frozen head bytes.
    #[must_use]
    pub fn bytes(&self) -> &Bytes {
        &self.head
    }

    /// The structural view of the head.
    #[must_use]
    pub fn head(&self) -> &HeadView {
        &self.view
    }

    /// The inline body, if one was attached.
    #[must_use]
    pub fn body(&self) -> Option<&Bytes> {
        self.body.as_ref()
    }

    /// Diagnostics raised while parsing the head.
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Whether this is a request or a response.
    #[must_use]
    pub fn kind(&self) -> Kind {
        self.kind
    }
}

/// The request method, as far as response framing cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Method {
    /// `HEAD`.
    Head,
    /// Any other method.
    Other,
}

/// What the parser must know about the request to frame its response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResponseContext {
    /// The request method.
    pub method: Method,
    /// Whether the request was `CONNECT`.
    pub is_connect: bool,
}

impl ResponseContext {
    /// Builds a context from the request method bytes. Matching is exact and
    /// case-sensitive.
    #[must_use]
    pub fn for_method(method: &[u8]) -> ResponseContext {
        ResponseContext {
            method: if method == b"HEAD" {
                Method::Head
            } else {
                Method::Other
            },
            is_connect: method == b"CONNECT",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sp(a: u32, b: u32) -> Span {
        Span::new(a, b)
    }

    fn view(buf: &[u8]) -> HeadView {
        // Hand-built headers: "Host: x\r\n" then "host: y\r\n".
        let _ = buf;
        let start = StartLine::Request {
            method: sp(0, 0),
            target: sp(0, 0),
            version: sp(0, 0),
        };
        let headers = alloc::vec![
            HeaderSpan::new(sp(0, 4), sp(6, 7), sp(0, 9)),
            HeaderSpan::new(sp(9, 13), sp(15, 16), sp(9, 18)),
        ];
        HeadView::new(start, headers, BodyFraming::None)
    }

    const BUF: &[u8] = b"Host: x\r\nhost: y\r\n";

    #[test]
    fn header_is_case_insensitive() {
        assert_eq!(view(BUF).header(BUF, "HOST"), Some(sp(6, 7)));
    }

    #[test]
    fn header_returns_first_duplicate() {
        assert_eq!(view(BUF).header(BUF, "host"), Some(sp(6, 7)));
    }

    #[test]
    fn header_missing_is_none() {
        assert_eq!(view(BUF).header(BUF, "accept"), None);
    }

    #[test]
    fn header_empty_name_is_none() {
        assert_eq!(view(BUF).header(BUF, ""), None);
    }

    #[test]
    fn header_out_of_range_span_is_none() {
        assert_eq!(view(BUF).header(b"Ho", "host"), None);
    }

    #[test]
    fn raw_message_accessors_and_inline_body() {
        let head = Bytes::from_static(BUF);
        let m = RawMessage::new(head, view(BUF), Kind::Request, Vec::new());
        assert_eq!(m.kind(), Kind::Request);
        assert!(m.body().is_none());
        assert!(m.diagnostics().is_empty());
        let m = m.with_inline_body(Bytes::from_static(b"hi"));
        assert_eq!(m.body().map(Bytes::as_ref), Some(&b"hi"[..]));
        assert_eq!(m.bytes().as_ref(), BUF);
    }

    #[test]
    fn for_method_matching() {
        assert_eq!(ResponseContext::for_method(b"HEAD").method, Method::Head);
        assert_eq!(ResponseContext::for_method(b"head").method, Method::Other);
        let c = ResponseContext::for_method(b"CONNECT");
        assert!(c.is_connect);
        assert_eq!(c.method, Method::Other);
        assert!(!ResponseContext::for_method(b"connect").is_connect);
        assert!(!ResponseContext::for_method(b"GET").is_connect);
    }
}
