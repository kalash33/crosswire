//! Reading one HTTP/1 response, following the `crosswire-h1` re-present
//! contract.

use std::time::Instant;

use bytes::{Buf, Bytes};
use crosswire_core::Diagnostic;
use crosswire_h1::{BodyFraming, HeadView, Progress, RawMessage, ResponseContext, StartLine};
use tokio::io::AsyncReadExt;

use crate::error::ClientError;
use crate::session::Session;

/// How much spare room one read gets. `pending` never holds more than the
/// parser's token limits plus one of these.
const READ_CHUNK: usize = 16 * 1024;

/// One piece of a response body, as the parser produced it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BodyPiece {
    /// The bytes as on the wire (chunk framing included).
    pub raw: Bytes,
    /// The decoded payload bytes.
    pub data: Bytes,
    /// The extension of the chunk this piece belongs to, when it had one.
    pub ext: Option<Bytes>,
    /// The trailer fields as `(name, value)`. They arrive after the last
    /// payload, so they ride on a final piece with empty `raw` and `data`.
    pub trailers: Option<Vec<(Bytes, Bytes)>>,
}

/// What the parser reported, copied out of the read buffer.
enum Ev {
    Head(RawMessage),
    Body { raw: Bytes, data: Bytes },
    Ext(Bytes),
    Trailers(Vec<(Bytes, Bytes)>),
    Done,
}

/// What the `Connection` header and HTTP version of a head say.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CloseSemantics {
    close: bool,
    keep_alive: bool,
    http10: bool,
}

impl CloseSemantics {
    /// Whether the sender expects the connection to close afterwards.
    pub(crate) fn closes(self) -> bool {
        self.close || (self.http10 && !self.keep_alive)
    }
}

fn trim_ws(mut b: &[u8]) -> &[u8] {
    while let [first, rest @ ..] = b {
        if *first == b' ' || *first == b'\t' {
            b = rest;
        } else {
            break;
        }
    }
    while let [rest @ .., last] = b {
        if *last == b' ' || *last == b'\t' {
            b = rest;
        } else {
            break;
        }
    }
    b
}

/// Reads every `Connection` header (case-insensitively, token lists
/// included) and the HTTP version of `msg`.
pub(crate) fn close_semantics(msg: &RawMessage) -> CloseSemantics {
    let buf = msg.bytes();
    let view: &HeadView = msg.head();
    let mut out = CloseSemantics::default();
    let version = match view.start() {
        StartLine::Request { version, .. } | StartLine::Response { version, .. } => {
            buf.get(version.range())
        }
        _ => None,
    };
    out.http10 = version.is_some_and(|v| trim_ws(v) == b"HTTP/1.0");
    for h in view.headers() {
        let (Some(name), Some(value)) = (buf.get(h.name().range()), buf.get(h.value().range()))
        else {
            continue;
        };
        if !trim_ws(name).eq_ignore_ascii_case(b"connection") {
            continue;
        }
        for token in value.split(|&b| b == b',') {
            let token = trim_ws(token);
            out.close |= token.eq_ignore_ascii_case(b"close");
            out.keep_alive |= token.eq_ignore_ascii_case(b"keep-alive");
        }
    }
    out
}

/// Whether `framing` ends the connection's usefulness: the body runs to
/// close, or the connection was upgraded.
fn ends_connection(framing: &BodyFraming) -> (bool, bool) {
    match framing {
        BodyFraming::UntilClose => (true, false),
        BodyFraming::Upgrade => (false, true),
        BodyFraming::Ambiguous { resolved, .. } => ends_connection(resolved),
        _ => (false, false),
    }
}

/// The response to one request, read incrementally.
///
/// Created by [`Session::send`]. Call [`head`](ResponseStream::head) and then
/// [`next_body`](ResponseStream::next_body) until it returns `None`. The
/// session is reusable only after the response was read to its end.
#[derive(Debug)]
pub struct ResponseStream<'a> {
    session: &'a mut Session,
    request: Vec<u8>,
    request_closes: bool,
    written: Instant,
    ttfb_ms: Option<u32>,
    /// Every response byte the parser consumed, in order.
    raw: Vec<u8>,
    raw_len: u64,
    raw_truncated_at: Option<u64>,
    ctx: ResponseContext,
    interim: Vec<RawMessage>,
    head: Option<RawMessage>,
    until_close: bool,
    ends_in_upgrade: bool,
    response_closes: bool,
    eof: bool,
    finished: bool,
    ext: Option<Bytes>,
    trailers: Option<Vec<(Bytes, Bytes)>>,
}

fn protocol(e: impl std::fmt::Display) -> ClientError {
    ClientError::Protocol(e.to_string())
}

impl<'a> ResponseStream<'a> {
    pub(crate) fn new(
        session: &'a mut Session,
        request: Vec<u8>,
        request_closes: bool,
        written: Instant,
        ctx: ResponseContext,
    ) -> Self {
        ResponseStream {
            session,
            request,
            request_closes,
            written,
            ttfb_ms: None,
            raw: Vec::new(),
            raw_len: 0,
            raw_truncated_at: None,
            ctx,
            interim: Vec::new(),
            head: None,
            until_close: false,
            ends_in_upgrade: false,
            response_closes: false,
            eof: false,
            finished: false,
            ext: None,
            trailers: None,
        }
    }

    /// The exact bytes written for the request.
    #[must_use]
    pub fn request_bytes(&self) -> &[u8] {
        &self.request
    }

    /// Milliseconds from the last request byte written to the first response
    /// byte read; 0 until a byte has been read.
    #[must_use]
    pub fn ttfb_ms(&self) -> u32 {
        self.ttfb_ms.unwrap_or(0)
    }

    /// The response bytes the parser has consumed so far, in order, interim
    /// responses included. Bytes read beyond the end of this response are not
    /// included. This is the retained prefix: it stops growing at the
    /// session's raw cap (see [`raw_len`](ResponseStream::raw_len) and
    /// [`raw_truncated_at`](ResponseStream::raw_truncated_at)).
    #[must_use]
    pub fn raw_response(&self) -> &[u8] {
        &self.raw
    }

    /// Total response bytes consumed so far, retained or not.
    #[must_use]
    pub fn raw_len(&self) -> u64 {
        self.raw_len
    }

    /// The length at which [`raw_response`](ResponseStream::raw_response)
    /// stopped growing, if the cap was reached and bytes were dropped.
    #[must_use]
    pub fn raw_truncated_at(&self) -> Option<u64> {
        self.raw_truncated_at
    }

    /// The interim (1xx other than 101) responses skipped before the final
    /// head, in order.
    #[must_use]
    pub fn interim_responses(&self) -> &[RawMessage] {
        &self.interim
    }

    fn push_raw(&mut self, bytes: &[u8]) {
        self.raw_len += bytes.len() as u64;
        let room = self.session.raw_cap.saturating_sub(self.raw.len());
        let take = room.min(bytes.len());
        self.raw.extend_from_slice(&bytes[..take]);
        if take < bytes.len() && self.raw_truncated_at.is_none() {
            self.raw_truncated_at = Some(self.raw.len() as u64);
        }
    }

    /// Diagnostics found in the body framing so far.
    #[must_use]
    pub fn body_diagnostics(&self) -> &[Diagnostic] {
        self.session.parser.body_diagnostics()
    }

    /// Reads until the final response head is complete and returns it,
    /// skipping interim 1xx responses (recorded in
    /// [`interim_responses`](ResponseStream::interim_responses)); `101` is a
    /// final head. Calling it again returns the same head.
    ///
    /// # Errors
    /// [`ClientError::Timeout`] when a read stalls, [`ClientError::Io`] on a
    /// socket error, [`ClientError::Protocol`] when the head is malformed
    /// beyond the parser limits or the connection closes first.
    pub async fn head(&mut self) -> Result<RawMessage, ClientError> {
        if let Some(h) = &self.head {
            return Ok(h.clone());
        }
        loop {
            let Ev::Head(msg) = self.next_event().await? else {
                return Err(ClientError::Protocol("body before head".into()));
            };
            let status = msg.head().status_code(msg.bytes());
            if matches!(status, Some(100..=199)) && status != Some(101) {
                self.interim.push(msg);
                self.session.parser.reset_response(self.ctx);
                continue;
            }
            {
                let (until_close, upgrade) = ends_connection(msg.head().framing());
                self.until_close = until_close;
                self.ends_in_upgrade = upgrade;
                self.response_closes = close_semantics(&msg).closes();
                self.head = Some(msg.clone());
                return Ok(msg);
            }
        }
    }

    /// Reads the next body piece; `None` once the response is complete.
    ///
    /// Reads the head first when [`head`](ResponseStream::head) was not
    /// called.
    ///
    /// # Errors
    /// As for [`head`](ResponseStream::head); a connection that closes
    /// before a length-delimited or chunked body ends is a
    /// [`ClientError::Protocol`].
    pub async fn next_body(&mut self) -> Result<Option<BodyPiece>, ClientError> {
        if self.head.is_none() {
            self.head().await?;
        }
        loop {
            if self.finished {
                return Ok(None);
            }
            match self.next_event().await? {
                Ev::Body { raw, data } => {
                    return Ok(Some(BodyPiece {
                        raw,
                        data,
                        ext: self.ext.take(),
                        trailers: None,
                    }))
                }
                Ev::Ext(e) => self.ext = Some(e),
                Ev::Trailers(t) => self.trailers = Some(t),
                Ev::Done => {
                    self.finish();
                    if self.ext.is_some() || self.trailers.is_some() {
                        return Ok(Some(BodyPiece {
                            raw: Bytes::new(),
                            data: Bytes::new(),
                            ext: self.ext.take(),
                            trailers: self.trailers.take(),
                        }));
                    }
                    return Ok(None);
                }
                Ev::Head(_) => return Err(ClientError::Protocol("second head".into())),
            }
        }
    }

    /// Marks the exchange complete and decides whether the connection can
    /// carry another.
    fn finish(&mut self) {
        self.finished = true;
        self.session.in_flight = false;
        self.session.reusable = !self.until_close
            && !self.ends_in_upgrade
            && !self.request_closes
            && !self.response_closes
            && self.session.pending.is_empty();
    }

    /// The reference loop of the `crosswire-h1` README, one event at a time:
    /// feed the unconsumed bytes, advance by `consumed`, read more only on
    /// `NeedMore`, and let an event that consumed nothing (`ChunkExt`, `Done`)
    /// be followed by a call with the same bytes.
    async fn next_event(&mut self) -> Result<Ev, ClientError> {
        loop {
            // Feeding even when `pending` is empty: a bodyless message is
            // already `Done`, and must not wait for a read.
            let step = self
                .session
                .parser
                .feed(&self.session.pending)
                .map_err(protocol)?;
            let input = &self.session.pending[..];
            let ev = match step.event {
                Progress::NeedMore => None,
                Progress::Head(msg) => Some(Ev::Head(msg)),
                Progress::BodyData { raw, data } => Some(Ev::Body {
                    raw: Bytes::copy_from_slice(&input[raw.range()]),
                    data: Bytes::copy_from_slice(&input[data.range()]),
                }),
                Progress::ChunkExt(s) => Some(Ev::Ext(Bytes::copy_from_slice(&input[s.range()]))),
                Progress::Trailers(fields) => Some(Ev::Trailers(
                    fields
                        .iter()
                        .map(|f| {
                            (
                                Bytes::copy_from_slice(&input[f.name().range()]),
                                Bytes::copy_from_slice(&input[f.value().range()]),
                            )
                        })
                        .collect(),
                )),
                Progress::Done => Some(Ev::Done),
                _ => return Err(ClientError::Protocol("unknown parser event".into())),
            };
            let consumed = step.consumed;
            self.push_raw_from_pending(consumed);
            self.session.pending.advance(consumed);
            if let Some(ev) = ev {
                return Ok(ev);
            }
            if let Some(ev) = self.read_more().await? {
                return Ok(ev);
            }
        }
    }

    fn push_raw_from_pending(&mut self, n: usize) {
        let chunk = self.session.pending[..n].to_vec();
        self.push_raw(&chunk);
    }

    /// Reads into `pending` after the unconsumed tail. At end of input,
    /// `feed_eof` completes an until-close body; any other message is
    /// truncated.
    async fn read_more(&mut self) -> Result<Option<Ev>, ClientError> {
        if self.eof {
            return self.at_eof();
        }
        self.session.pending.reserve(READ_CHUNK);
        let timeout = self.session.read_timeout;
        let read = self
            .session
            .conn
            .io_mut()
            .read_buf(&mut self.session.pending);
        let n = tokio::time::timeout(timeout, read)
            .await
            .map_err(|_| ClientError::Timeout)??;
        if n == 0 {
            self.eof = true;
            return self.at_eof();
        }
        if self.ttfb_ms.is_none() {
            self.ttfb_ms =
                Some(u32::try_from(self.written.elapsed().as_millis()).unwrap_or(u32::MAX));
        }
        Ok(None)
    }

    fn at_eof(&mut self) -> Result<Option<Ev>, ClientError> {
        let step = self.session.parser.feed_eof().map_err(protocol)?;
        match step.event {
            Progress::Done => Ok(Some(Ev::Done)),
            _ => Err(ClientError::Protocol("unexpected end of input".into())),
        }
    }
}
