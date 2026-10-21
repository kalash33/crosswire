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
    #[allow(dead_code)] // used by the parser (Tasks 3-7)
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
    #[allow(dead_code)] // used by the parser (Tasks 3-7)
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
    #[must_use]
    pub fn header(&self, buf: &[u8], name: &str) -> Option<Span> {
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
    #[allow(dead_code)] // used by the parser (Tasks 3-7)
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
