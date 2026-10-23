//! The `Message` schema (spec section 6.1) and its parts.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use bytes::Bytes;
use crosswire_core::{Change, ChangeOp, ChangeReason, Diagnostic, Span};
use crosswire_h1::{BodyFraming, HeadView, RawMessage, ResolutionRule, StartLine};
use serde::{Deserialize, Serialize};

use crate::{Fidelity, MessageKind};

/// Version of the HTTP/1 parser whose spans a stored
/// [`Head::Http1Raw`] carries. Spans and diagnostics are re-derived on read
/// when the stored value differs from the running parser's.
pub const HTTP1_PARSER_VERSION: u32 = 1;

/// The start line of an HTTP/1 head, as spans into the head bytes. A
/// serde-able mirror of [`crosswire_h1::StartLine`].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum StartLineSpans {
    /// A request line.
    Request {
        /// The method token.
        method: Span,
        /// The request target.
        target: Span,
        /// The version text.
        version: Span,
    },
    /// A status line.
    Response {
        /// The version text.
        version: Span,
        /// The status code text.
        status: Span,
        /// The reason phrase (possibly empty).
        reason: Span,
    },
}

/// One header field's spans. A serde-able mirror of
/// [`crosswire_h1::HeaderSpan`].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeaderSpanRecord {
    name: Span,
    value: Span,
    line: Span,
}

impl HeaderSpanRecord {
    /// Creates a record.
    #[must_use]
    pub fn new(name: Span, value: Span, line: Span) -> HeaderSpanRecord {
        HeaderSpanRecord { name, value, line }
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

    /// The whole line including its terminator.
    #[must_use]
    pub fn line(&self) -> Span {
        self.line
    }
}

/// The resolved body framing. A serde-able mirror of
/// [`crosswire_h1::BodyFraming`] that stores the resolution rule by its
/// stable name, so the stored form does not depend on the h1 crate's enum.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum FramingRecord {
    /// No body.
    None,
    /// A fixed number of bytes.
    ContentLength {
        /// The declared length.
        len: u64,
    },
    /// Chunked transfer coding.
    Chunked,
    /// The body runs until the connection closes.
    UntilClose,
    /// The connection was upgraded; no HTTP body follows.
    Upgrade,
    /// Conflicting framing signals; one was chosen by a rule.
    Ambiguous {
        /// The framing that was chosen.
        resolved: Box<FramingRecord>,
        /// Every framing the message could be read as.
        #[serde(default)]
        candidates: Vec<FramingRecord>,
        /// Stable kebab-case name of the rule that picked `resolved`
        /// (`transfer-encoding-wins`, `first-content-length`,
        /// `no-body-status`, `head-method`, `connect-success`,
        /// `invalid-content-length`, or `unknown` for a rule newer than this
        /// crate).
        rule: String,
    },
}

fn rule_name(rule: &ResolutionRule) -> &'static str {
    match rule {
        ResolutionRule::TransferEncodingWins => "transfer-encoding-wins",
        ResolutionRule::FirstContentLength => "first-content-length",
        ResolutionRule::NoBodyStatus => "no-body-status",
        ResolutionRule::HeadMethod => "head-method",
        ResolutionRule::ConnectSuccess => "connect-success",
        ResolutionRule::InvalidContentLength => "invalid-content-length",
        _ => "unknown",
    }
}

impl From<&BodyFraming> for FramingRecord {
    fn from(f: &BodyFraming) -> FramingRecord {
        match f {
            BodyFraming::None => FramingRecord::None,
            BodyFraming::ContentLength(len) => FramingRecord::ContentLength { len: *len },
            BodyFraming::Chunked => FramingRecord::Chunked,
            BodyFraming::UntilClose => FramingRecord::UntilClose,
            BodyFraming::Upgrade => FramingRecord::Upgrade,
            BodyFraming::Ambiguous {
                resolved,
                candidates,
                rule,
            } => FramingRecord::Ambiguous {
                resolved: Box::new(FramingRecord::from(&**resolved)),
                candidates: candidates.iter().map(FramingRecord::from).collect(),
                rule: String::from(rule_name(rule)),
            },
            // A framing newer than this crate: the closest honest record.
            _ => FramingRecord::None,
        }
    }
}

/// Spans of an HTTP/1 head: start line, header fields and the resolved
/// framing. A serde-able copy of [`crosswire_h1::HeadView`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeadSpans {
    start_line: StartLineSpans,
    #[serde(default)]
    headers: Vec<HeaderSpanRecord>,
    framing: FramingRecord,
}

impl HeadSpans {
    /// Creates head spans from parts.
    #[must_use]
    pub fn new(
        start_line: StartLineSpans,
        headers: Vec<HeaderSpanRecord>,
        framing: FramingRecord,
    ) -> HeadSpans {
        HeadSpans {
            start_line,
            headers,
            framing,
        }
    }

    /// The start line.
    #[must_use]
    pub fn start_line(&self) -> &StartLineSpans {
        &self.start_line
    }

    /// The header fields in wire order.
    #[must_use]
    pub fn headers(&self) -> &[HeaderSpanRecord] {
        &self.headers
    }

    /// The framing resolved when the head was parsed.
    #[must_use]
    pub fn framing(&self) -> &FramingRecord {
        &self.framing
    }
}

impl From<&HeadView> for HeadSpans {
    fn from(v: &HeadView) -> HeadSpans {
        let start_line = match v.start() {
            StartLine::Request {
                method,
                target,
                version,
            } => StartLineSpans::Request {
                method: *method,
                target: *target,
                version: *version,
            },
            StartLine::Response {
                version,
                status,
                reason,
            } => StartLineSpans::Response {
                version: *version,
                status: *status,
                reason: *reason,
            },
            // A start-line shape newer than this crate: an empty request line.
            _ => StartLineSpans::Request {
                method: Span::new(0, 0),
                target: Span::new(0, 0),
                version: Span::new(0, 0),
            },
        };
        HeadSpans {
            start_line,
            headers: v
                .headers()
                .iter()
                .map(|h| HeaderSpanRecord::new(h.name(), h.value(), h.line()))
                .collect(),
            framing: FramingRecord::from(v.framing()),
        }
    }
}

/// Why a connection became an opaque tunnel.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum TunnelReason {
    /// A WebSocket upgrade (v0.1 relays frames unparsed).
    Ws,
    /// The host is on the SNI passthrough list.
    SniPassthrough,
    /// The host was auto-added after repeated handshake rejections.
    AutoPassthrough,
    /// The stream could not be delimited.
    Undelimitable,
    /// A reason written by a newer version of Crosswire.
    #[serde(other)]
    Unknown,
}

/// The head of a message.
///
/// Serialised internally tagged by `type`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
#[non_exhaustive]
pub enum Head {
    /// HTTP/1 head bytes exactly as on the wire, with parser spans.
    Http1Raw {
        /// The head bytes, including the terminating empty line.
        #[serde(with = "crate::bytes_serde")]
        bytes: Bytes,
        /// Spans into `bytes`.
        spans: HeadSpans,
        /// The parser version that produced `spans`
        /// ([`HTTP1_PARSER_VERSION`]).
        parser_version: u32,
    },
    /// A translated header list (frames absent, hints null).
    Normalized {
        /// Header name and value pairs in order.
        #[serde(default, with = "crate::bytes_serde::pairs")]
        map: Vec<(String, Vec<u8>)>,
    },
    /// An opaque tunnel.
    Tunnel {
        /// Bytes relayed client to upstream.
        bytes_in: u64,
        /// Bytes relayed upstream to client.
        bytes_out: u64,
        /// Tunnel lifetime in milliseconds.
        duration_ms: u64,
        /// Why the connection is tunnelled.
        reason: TunnelReason,
    },
}

/// The length and SHA-256 of a whole body as it passed through the relay
/// path, recorded when the stored capture is not the whole body (cut off at
/// the capture limit, or dropped because the capture window was full).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Capture {
    /// Bytes relayed.
    pub len: u64,
    /// SHA-256 of the relayed bytes.
    #[serde(with = "crate::bytes_serde::hash")]
    pub sha256: [u8; 32],
}

/// The body of a message.
///
/// Serialised internally tagged by `type`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
#[non_exhaustive]
pub enum Body {
    /// No body.
    None,
    /// A body held inline (raw framed bytes, as on the wire). Build one
    /// with [`Body::inline`] when only the bytes are known.
    Inline {
        /// The bytes.
        #[serde(with = "crate::bytes_serde")]
        bytes: Bytes,
        /// If the capture was cut off, the number of bytes kept.
        #[serde(default)]
        truncated_at: Option<u64>,
        /// The capture window was full; length and hash come from `relay`.
        #[serde(default)]
        capture_dropped: bool,
        /// Length and hash of the whole body on the relay path, when the
        /// capture is not the whole body.
        #[serde(default)]
        relay: Option<Capture>,
    },
    /// A body stored in the blob store.
    Blob {
        /// SHA-256 of the full body.
        #[serde(with = "crate::bytes_serde::hash")]
        hash: [u8; 32],
        /// Full body length in bytes.
        len: u64,
        /// If the capture was cut off, the number of bytes kept.
        #[serde(default)]
        truncated_at: Option<u64>,
        /// The capture window was full; length and hash come from the relay.
        #[serde(default)]
        capture_dropped: bool,
        /// Length and hash of the whole body on the relay path, when the
        /// capture is not the whole body.
        #[serde(default)]
        relay: Option<Capture>,
    },
}

impl Body {
    /// An inline body holding exactly `bytes`: not truncated, not dropped,
    /// no separate relay record.
    #[must_use]
    pub fn inline(bytes: Bytes) -> Body {
        Body::Inline {
            bytes,
            truncated_at: None,
            capture_dropped: false,
            relay: None,
        }
    }
}

/// A recorded edit to a message (`ChangeOp` and `ChangeReason` come from
/// `crosswire-core`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diff {
    op: ChangeOp,
    span: Span,
    #[serde(with = "crate::bytes_serde")]
    before: Bytes,
    #[serde(with = "crate::bytes_serde")]
    after: Bytes,
    reason: ChangeReason,
}

impl Diff {
    /// Creates a diff.
    #[must_use]
    pub fn new(
        op: ChangeOp,
        span: Span,
        before: Bytes,
        after: Bytes,
        reason: ChangeReason,
    ) -> Diff {
        Diff {
            op,
            span,
            before,
            after,
            reason,
        }
    }

    /// The kind of edit.
    #[must_use]
    pub fn op(&self) -> ChangeOp {
        self.op
    }

    /// The affected span in the original message.
    #[must_use]
    pub fn span(&self) -> Span {
        self.span
    }

    /// Bytes before the edit.
    #[must_use]
    pub fn before(&self) -> &Bytes {
        &self.before
    }

    /// Bytes after the edit.
    #[must_use]
    pub fn after(&self) -> &Bytes {
        &self.after
    }

    /// Why the edit was made.
    #[must_use]
    pub fn reason(&self) -> &ChangeReason {
        &self.reason
    }
}

impl From<&Change> for Diff {
    fn from(c: &Change) -> Diff {
        Diff::new(
            c.op(),
            c.span(),
            c.before().clone(),
            c.after().clone(),
            c.reason().clone(),
        )
    }
}

/// Result of validating the upstream TLS chain.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
#[non_exhaustive]
pub enum ChainResult {
    /// The connection is not TLS.
    #[default]
    NotTls,
    /// The chain validated.
    Valid,
    /// The chain failed validation.
    Invalid {
        /// Why it failed.
        #[serde(default)]
        reason: String,
    },
    /// Validation failure was overridden for this host.
    Overridden {
        /// The validation error that was overridden (empty in data written
        /// before the field existed).
        #[serde(default)]
        reason: String,
    },
}

/// Connection facts recorded with each message.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ConnectionInfo {
    target_addr: String,
    port: u16,
    peer_addr: Option<String>,
    client_addr: Option<String>,
    sni: Option<String>,
    alpn: Option<String>,
    tls_version: Option<String>,
    cipher: Option<String>,
    upstream_chain_result: ChainResult,
    cert_override: bool,
    conn_id: u64,
    reused: bool,
}

impl ConnectionInfo {
    /// Creates a record for a target; everything else starts empty.
    ///
    /// `target_addr` is the host as configured or requested (a name or an
    /// address literal, spec section 4.6), not the address it resolved to;
    /// record that with [`ConnectionInfo::with_peer_addr`].
    #[must_use]
    pub fn new(target_addr: impl Into<String>, port: u16) -> ConnectionInfo {
        ConnectionInfo {
            target_addr: target_addr.into(),
            port,
            ..ConnectionInfo::default()
        }
    }

    /// Sets the address actually connected to (the resolved IP).
    #[must_use]
    pub fn with_peer_addr(mut self, v: impl Into<String>) -> Self {
        self.peer_addr = Some(v.into());
        self
    }

    /// Sets the client's address.
    #[must_use]
    pub fn with_client_addr(mut self, v: impl Into<String>) -> Self {
        self.client_addr = Some(v.into());
        self
    }

    /// Sets the SNI sent upstream.
    #[must_use]
    pub fn with_sni(mut self, v: impl Into<String>) -> Self {
        self.sni = Some(v.into());
        self
    }

    /// Sets the negotiated ALPN protocol.
    #[must_use]
    pub fn with_alpn(mut self, v: impl Into<String>) -> Self {
        self.alpn = Some(v.into());
        self
    }

    /// Sets the negotiated TLS version and cipher.
    #[must_use]
    pub fn with_tls(mut self, version: impl Into<String>, cipher: impl Into<String>) -> Self {
        self.tls_version = Some(version.into());
        self.cipher = Some(cipher.into());
        self
    }

    /// Sets the upstream chain validation result.
    #[must_use]
    pub fn with_chain_result(mut self, v: ChainResult) -> Self {
        self.upstream_chain_result = v;
        self
    }

    /// Marks a per-host certificate override.
    #[must_use]
    pub fn with_cert_override(mut self, v: bool) -> Self {
        self.cert_override = v;
        self
    }

    /// Sets the connection id and whether the connection was reused.
    #[must_use]
    pub fn with_conn(mut self, conn_id: u64, reused: bool) -> Self {
        self.conn_id = conn_id;
        self.reused = reused;
        self
    }

    /// The connection target as configured: a host name or an address
    /// literal (spec section 4.6).
    #[must_use]
    pub fn target_addr(&self) -> &str {
        &self.target_addr
    }

    /// The address actually connected to, if recorded.
    #[must_use]
    pub fn peer_addr(&self) -> Option<&str> {
        self.peer_addr.as_deref()
    }

    /// The target port.
    #[must_use]
    pub fn port(&self) -> u16 {
        self.port
    }

    /// The client's address.
    #[must_use]
    pub fn client_addr(&self) -> Option<&str> {
        self.client_addr.as_deref()
    }

    /// The SNI used.
    #[must_use]
    pub fn sni(&self) -> Option<&str> {
        self.sni.as_deref()
    }

    /// The negotiated ALPN protocol.
    #[must_use]
    pub fn alpn(&self) -> Option<&str> {
        self.alpn.as_deref()
    }

    /// The negotiated TLS version.
    #[must_use]
    pub fn tls_version(&self) -> Option<&str> {
        self.tls_version.as_deref()
    }

    /// The negotiated cipher suite.
    #[must_use]
    pub fn cipher(&self) -> Option<&str> {
        self.cipher.as_deref()
    }

    /// The upstream chain validation result.
    #[must_use]
    pub fn upstream_chain_result(&self) -> &ChainResult {
        &self.upstream_chain_result
    }

    /// Whether a per-host certificate override applied.
    #[must_use]
    pub fn cert_override(&self) -> bool {
        self.cert_override
    }

    /// The connection id.
    #[must_use]
    pub fn conn_id(&self) -> u64 {
        self.conn_id
    }

    /// Whether the upstream connection was reused.
    #[must_use]
    pub fn reused(&self) -> bool {
        self.reused
    }
}

/// One recorded request or response.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Message {
    kind: MessageKind,
    fidelity: Fidelity,
    head: Head,
    framing: FramingRecord,
    #[serde(default)]
    diagnostics: Vec<Diagnostic>,
    body: Body,
    #[serde(default)]
    diffs: Vec<Diff>,
    #[serde(default)]
    connection: ConnectionInfo,
    /// Interim (1xx) heads forwarded before the final response, raw bytes.
    #[serde(default, with = "crate::bytes_serde::vec")]
    interim: Vec<Bytes>,
}

impl Message {
    /// Creates a message from parts, with no diagnostics or diffs.
    #[must_use]
    pub fn new(
        kind: MessageKind,
        fidelity: Fidelity,
        head: Head,
        framing: FramingRecord,
        body: Body,
        connection: ConnectionInfo,
    ) -> Message {
        Message {
            kind,
            fidelity,
            head,
            framing,
            diagnostics: Vec::new(),
            body,
            diffs: Vec::new(),
            connection,
            interim: Vec::new(),
        }
    }

    /// Records a parsed HTTP/1 head as a raw message: head bytes copied,
    /// spans and framing from the head view, diagnostics from the message,
    /// `kind = Http1Raw`, `fidelity = Raw`. Any inline body attached to
    /// `head` is ignored; pass the body to record as `body`.
    #[must_use]
    pub fn http1_raw(head: &RawMessage, body: Body, connection: ConnectionInfo) -> Message {
        let spans = HeadSpans::from(head.head());
        let framing = spans.framing().clone();
        Message {
            kind: MessageKind::Http1Raw,
            fidelity: Fidelity::Raw,
            head: Head::Http1Raw {
                bytes: head.bytes().clone(),
                spans,
                parser_version: HTTP1_PARSER_VERSION,
            },
            framing,
            diagnostics: head.diagnostics().to_vec(),
            body,
            diffs: Vec::new(),
            connection,
            interim: Vec::new(),
        }
    }

    /// Sets the recorded diffs.
    #[must_use]
    pub fn with_diffs(mut self, diffs: Vec<Diff>) -> Message {
        self.diffs = diffs;
        self
    }

    /// Sets the interim (1xx) heads forwarded before this final response,
    /// as raw head bytes in the order they were sent.
    #[must_use]
    pub fn with_interim(mut self, interim: Vec<Bytes>) -> Message {
        self.interim = interim;
        self
    }

    /// Sets the diagnostics.
    #[must_use]
    pub fn with_diagnostics(mut self, diagnostics: Vec<Diagnostic>) -> Message {
        self.diagnostics = diagnostics;
        self
    }

    /// Sets the fidelity label.
    #[must_use]
    pub fn with_fidelity(mut self, fidelity: Fidelity) -> Message {
        self.fidelity = fidelity;
        self
    }

    /// Replaces the head and diagnostics (used by redaction, which re-derives
    /// both from rebuilt bytes).
    pub(crate) fn with_head(mut self, head: Head, diagnostics: Option<Vec<Diagnostic>>) -> Message {
        self.head = head;
        if let Some(d) = diagnostics {
            self.diagnostics = d;
        }
        self
    }

    /// The head representation.
    #[must_use]
    pub fn kind(&self) -> MessageKind {
        self.kind
    }

    /// How faithful the recorded bytes are.
    #[must_use]
    pub fn fidelity(&self) -> Fidelity {
        self.fidelity
    }

    /// The head.
    #[must_use]
    pub fn head(&self) -> &Head {
        &self.head
    }

    /// The resolved body framing.
    #[must_use]
    pub fn framing(&self) -> &FramingRecord {
        &self.framing
    }

    /// Diagnostics raised while parsing.
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// The body.
    #[must_use]
    pub fn body(&self) -> &Body {
        &self.body
    }

    /// Edits applied to the message.
    #[must_use]
    pub fn diffs(&self) -> &[Diff] {
        &self.diffs
    }

    /// Connection facts.
    #[must_use]
    pub fn connection(&self) -> &ConnectionInfo {
        &self.connection
    }

    /// Interim (1xx) heads forwarded before the final response, raw bytes.
    #[must_use]
    pub fn interim(&self) -> &[Bytes] {
        &self.interim
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crosswire_h1::{Parser, ParserConfig, Progress, ResponseContext};

    fn head(p: &mut Parser, input: &[u8]) -> RawMessage {
        match p.feed(input).unwrap().event {
            Progress::Head(m) => m,
            other => panic!("expected head, got {other:?}"),
        }
    }

    #[test]
    fn head_spans_from_request_view() {
        let buf = b"POST /a HTTP/1.1\r\nHost: x\r\nContent-Length: 3\r\n\r\n";
        let raw = head(&mut Parser::request(&ParserConfig::default()), buf);
        let s = HeadSpans::from(raw.head());
        let StartLineSpans::Request {
            method,
            target,
            version,
        } = *s.start_line()
        else {
            panic!("request expected")
        };
        assert_eq!(&buf[method.range()], b"POST");
        assert_eq!(&buf[target.range()], b"/a");
        assert_eq!(&buf[version.range()], b"HTTP/1.1");
        assert_eq!(s.headers().len(), 2);
        assert_eq!(&buf[s.headers()[1].name().range()], b"Content-Length");
        assert_eq!(&buf[s.headers()[1].value().range()], b"3");
        assert_eq!(&buf[s.headers()[0].line().range()], b"Host: x\r\n");
        assert_eq!(*s.framing(), FramingRecord::ContentLength { len: 3 });
    }

    #[test]
    fn head_spans_from_response_with_ambiguous_framing() {
        let buf = b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nTransfer-Encoding: chunked\r\n\r\n";
        let mut p = Parser::response(
            &ParserConfig::default(),
            ResponseContext::for_method(b"GET"),
        );
        let raw = head(&mut p, buf);
        let s = HeadSpans::from(raw.head());
        let StartLineSpans::Response { status, reason, .. } = *s.start_line() else {
            panic!("response expected")
        };
        assert_eq!(&buf[status.range()], b"200");
        assert_eq!(&buf[reason.range()], b"OK");
        let FramingRecord::Ambiguous {
            resolved,
            candidates,
            rule,
        } = s.framing()
        else {
            panic!("ambiguous expected, got {:?}", s.framing())
        };
        assert_eq!(**resolved, FramingRecord::Chunked);
        assert!(candidates.contains(&FramingRecord::ContentLength { len: 4 }));
        assert_eq!(rule, "transfer-encoding-wins");
    }
}
