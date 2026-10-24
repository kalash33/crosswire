//! The streaming, sans-IO message parser.

mod diag;
mod framing;
mod head;

pub(crate) use diag::Diagnostics;
pub(crate) use head::{find_line_end, is_leading_ws, parse_status, split_header};

use bytes::BytesMut;

use crate::chunked::{BodyDecoder, BodyEvent};
use crate::config::ParserConfig;
use crate::error::ParseError;
use crate::limits::ParseLimits;
use crate::message::{BodyFraming, HeadView, Kind, RawMessage, ResponseContext};
use crate::progress::{Progress, Step};
use crosswire_core::{Diagnostic, Profile, Span};

/// Scan state: still skipping whitespace before the start line.
const LEADING: u8 = 0;
/// Scan state: inside a line that already has content.
const IN_LINE: u8 = 1;
/// Scan state: at the start of a line (just after a line terminator).
const LINE_START: u8 = 2;
/// Scan state: at the start of a line and a `\r` has been seen.
const LINE_START_CR: u8 = 3;

/// The bounded buffer a head is accumulated in. It never holds more than
/// `head_size` bytes and never allocates capacity beyond that: input is
/// checked before it is copied in, and growth is clamped to the limit.
#[derive(Debug)]
struct HeadBuf {
    bytes: BytesMut,
    limit: usize,
    /// Where the incremental scan for the empty line currently stands.
    term: u8,
}

impl HeadBuf {
    fn new(limit: usize) -> Self {
        HeadBuf {
            bytes: BytesMut::with_capacity(limit.min(4096)),
            limit,
            term: LEADING,
        }
    }

    /// Returns how many bytes of `input` belong to the head (up to and
    /// including the empty line's terminator if it completes) and whether it
    /// completes. Line terminators are `\r\n` or a bare `\n`; whitespace
    /// before the start line is not part of the search.
    fn scan(&mut self, input: &[u8]) -> (usize, bool) {
        let mut st = self.term;
        for (i, &b) in input.iter().enumerate() {
            st = match (st, b) {
                (LEADING, b) if head::is_leading_ws(b) => LEADING,
                (IN_LINE, b'\n') => LINE_START,
                (LINE_START | LINE_START_CR, b'\n') => return (i + 1, true),
                (LINE_START, b'\r') => LINE_START_CR,
                _ => IN_LINE,
            };
        }
        self.term = st;
        (input.len(), false)
    }

    /// Appends `data`, growing capacity by at most what the limit allows.
    fn append(&mut self, data: &[u8]) {
        let need = self.bytes.len() + data.len();
        let cap = self.bytes.capacity();
        if need > cap {
            let target = need.max(cap.saturating_mul(2)).min(self.limit.max(need));
            let mut grown = BytesMut::with_capacity(target);
            grown.extend_from_slice(&self.bytes);
            self.bytes = grown;
        }
        self.bytes.extend_from_slice(data);
    }

    fn clear(&mut self) {
        self.bytes.clear();
        self.term = LEADING;
    }
}

#[derive(Debug)]
enum State {
    /// Accumulating the message head.
    Head,
    /// A body of known length; `remaining` bytes are still to come.
    Fixed { remaining: u64 },
    /// A chunked body, decoded by the parser's [`BodyDecoder`].
    Chunked,
    /// A body that runs until the end of the input.
    UntilClose,
    /// The message is complete.
    Done,
    /// A fatal error occurred; it is returned until [`Parser::reset`].
    Failed(ParseError),
}

/// The largest input prefix one body-phase call looks at, so that `consumed`
/// never exceeds what a [`Span`] can describe.
const MAX_SPAN: usize = u32::MAX as usize;

/// An incremental HTTP/1 message parser.
///
/// Feed it bytes with [`feed`](Parser::feed) in a loop until it returns
/// [`Progress::NeedMore`]. After [`Progress::Done`] the parser keeps returning
/// `Done` (consuming nothing) until [`reset`](Parser::reset); the unconsumed
/// tail of the input is the next pipelined message.
#[derive(Debug)]
pub struct Parser {
    kind: Kind,
    limits: ParseLimits,
    profile: Profile,
    state: State,
    buf: HeadBuf,
    /// The request a response answers; `None` for request parsers.
    ctx: Option<ResponseContext>,
    /// Decodes chunked bodies; also holds their diagnostics.
    decoder: BodyDecoder,
}

impl Parser {
    /// Creates a parser for request messages.
    #[must_use]
    pub fn request(cfg: &ParserConfig) -> Parser {
        Parser {
            kind: Kind::Request,
            limits: *cfg.limits(),
            profile: cfg.profile(),
            state: State::Head,
            buf: HeadBuf::new(usize::try_from(cfg.limits().head_size).unwrap_or(usize::MAX)),
            ctx: None,
            decoder: BodyDecoder::with_profile(cfg.limits(), cfg.profile()),
        }
    }

    /// Creates a parser for response messages. `ctx` describes the request
    /// being answered, which decides how the body is framed.
    ///
    /// [`reset`](Parser::reset) keeps `ctx`. A keep-alive relay parsing the
    /// next response on the connection must call
    /// [`reset_response`](Parser::reset_response) with the context of the
    /// next request instead, or a response to `HEAD` or `CONNECT` is framed
    /// with the previous request's method.
    #[must_use]
    pub fn response(cfg: &ParserConfig, ctx: ResponseContext) -> Parser {
        Parser {
            kind: Kind::Response,
            ctx: Some(ctx),
            ..Parser::request(cfg)
        }
    }

    /// The request context of a response parser; `None` for a request parser.
    pub(crate) fn response_context(&self) -> Option<&ResponseContext> {
        self.ctx.as_ref()
    }

    /// Feeds input and returns how much was consumed and what it produced.
    ///
    /// The `Head` step consumes exactly the bytes up to the end of the blank
    /// line. Once a message is `Done`, further input is left unconsumed. After
    /// an error, every later call returns the same error until
    /// [`reset`](Parser::reset).
    ///
    /// Re-present contract: body-phase tokens (a chunk-size line with its
    /// extension, and the trailer block) are never buffered across calls. When
    /// one is incomplete the step is `NeedMore` with `consumed` at the token
    /// start, and the caller must present the unconsumed bytes again together
    /// with new data. Chunk data is consumed incrementally.
    ///
    /// # Errors
    ///
    /// - [`ParseError::HeadTooLarge`]: the head reached
    ///   [`ParseLimits::head_size`] without its empty line.
    /// - [`ParseError::LineTooLong`]: a head line exceeded
    ///   [`ParseLimits::max_line`], or a chunk-size line exceeded
    ///   [`ParseLimits::max_chunk_size_line`].
    /// - [`ParseError::TooManyHeaders`]: more than
    ///   [`ParseLimits::max_headers`] header fields.
    /// - [`ParseError::InvalidChunkSize`]: a chunk size that is not
    ///   hexadecimal, or a malformed line break after chunk data.
    /// - [`ParseError::TrailerTooLarge`]: the trailer block exceeded
    ///   [`ParseLimits::max_trailer_size`].
    /// - [`ParseError::TooManyChunks`]: more data chunks than
    ///   [`ParseLimits::max_chunks`].
    /// - The earlier error again, if the parser already failed; it stays
    ///   failed until [`reset`](Parser::reset).
    pub fn feed(&mut self, input: &[u8]) -> Result<Step, ParseError> {
        let result = match &self.state {
            State::Done => {
                return Ok(Step {
                    consumed: 0,
                    event: Progress::Done,
                })
            }
            State::Failed(e) => return Err(e.clone()),
            State::Head => self.feed_head(input),
            State::Fixed { remaining } => Ok(self.feed_fixed(*remaining, input)),
            State::Chunked => self.feed_chunked(input),
            State::UntilClose => Ok(feed_until_close(input)),
        };
        if let Err(e) = &result {
            self.state = State::Failed(e.clone());
        }
        result
    }

    /// Signals the end of the input stream.
    ///
    /// An idle parser (see [`is_idle`](Parser::is_idle)) also returns
    /// `UnexpectedEof`: a relay that sees the connection close between
    /// messages checks `is_idle()` first to tell a clean close from a
    /// truncated message.
    ///
    /// # Errors
    ///
    /// Returns [`ParseError::UnexpectedEof`] if the head or a fixed or chunked
    /// body is still incomplete, or the earlier error if the parser has
    /// already failed. A body that runs until close completes here.
    pub fn feed_eof(&mut self) -> Result<Step, ParseError> {
        match &self.state {
            State::Done => Ok(Step {
                consumed: 0,
                event: Progress::Done,
            }),
            State::Failed(e) => Err(e.clone()),
            State::UntilClose => {
                self.state = State::Done;
                Ok(Step {
                    consumed: 0,
                    event: Progress::Done,
                })
            }
            State::Head | State::Fixed { .. } | State::Chunked => {
                self.state = State::Failed(ParseError::UnexpectedEof);
                Err(ParseError::UnexpectedEof)
            }
        }
    }

    /// Diagnostics found while reading the body (chunk framing, trailers).
    /// They are cleared by [`reset`](Parser::reset).
    ///
    /// Spans are absolute offsets from the first body byte: byte 0 is the
    /// first byte after the head's empty line. They saturate at `u32::MAX`
    /// and do not depend on how the input was split across calls (unlike
    /// the spans in [`Progress`] events, which index each call's input).
    ///
    /// At most [`ParseLimits::max_body_diagnostics`] are kept per message;
    /// the rest are counted by
    /// [`body_diagnostics_dropped`](Parser::body_diagnostics_dropped).
    #[must_use]
    pub fn body_diagnostics(&self) -> &[Diagnostic] {
        self.decoder.diagnostics()
    }

    /// How many body diagnostics were found but not kept because
    /// [`ParseLimits::max_body_diagnostics`] was reached.
    #[must_use]
    pub fn body_diagnostics_dropped(&self) -> usize {
        self.decoder.diagnostics_dropped()
    }

    /// Whether the parser is between messages: in the head state with
    /// nothing buffered. True for a fresh parser and right after
    /// [`reset`](Parser::reset) or [`reset_response`](Parser::reset_response),
    /// until the first byte of a head (including skipped leading whitespace)
    /// is consumed.
    #[must_use]
    pub fn is_idle(&self) -> bool {
        matches!(self.state, State::Head) && self.buf.bytes.is_empty()
    }

    /// Discards all state and starts parsing a new message. A response
    /// parser keeps its [`ResponseContext`]; use
    /// [`reset_response`](Parser::reset_response) to replace it.
    pub fn reset(&mut self) {
        self.state = State::Head;
        self.buf.clear();
        self.decoder.reset();
    }

    /// Like [`reset`](Parser::reset), and also replaces the
    /// [`ResponseContext`] with `ctx`, the request the next response answers.
    /// On a request parser `ctx` is ignored.
    pub fn reset_response(&mut self, ctx: ResponseContext) {
        self.reset();
        if self.kind == Kind::Response {
            self.ctx = Some(ctx);
        }
    }

    fn feed_fixed(&mut self, remaining: u64, input: &[u8]) -> Step {
        // Capped so that `consumed` never exceeds what the span can describe.
        let avail = input.len().min(MAX_SPAN);
        let take = usize::try_from(remaining).map_or(avail, |r| r.min(avail));
        if take == 0 {
            return Step {
                consumed: 0,
                event: Progress::NeedMore,
            };
        }
        let left = remaining.saturating_sub(take as u64);
        self.state = if left == 0 {
            State::Done
        } else {
            State::Fixed { remaining: left }
        };
        debug_assert!(take <= MAX_SPAN);
        let whole = Span::new(0, u32::try_from(take).unwrap_or(u32::MAX));
        Step {
            consumed: take,
            event: Progress::BodyData {
                raw: whole,
                data: whole,
            },
        }
    }

    fn feed_chunked(&mut self, input: &[u8]) -> Result<Step, ParseError> {
        let (consumed, ev) = self.decoder.feed(input)?;
        if self.decoder.is_done() {
            self.state = State::Done;
        }
        let event = match ev {
            BodyEvent::NeedMore => Progress::NeedMore,
            BodyEvent::Ext(s) => Progress::ChunkExt(s),
            BodyEvent::Data { raw, data } => Progress::BodyData { raw, data },
            BodyEvent::Trailers(t) => Progress::Trailers(t),
            BodyEvent::Done => Progress::Done,
        };
        Ok(Step { consumed, event })
    }

    fn feed_head(&mut self, input: &[u8]) -> Result<Step, ParseError> {
        let (take, complete) = self.buf.scan(input);
        let limit = usize::try_from(self.limits.head_size).unwrap_or(usize::MAX);
        if self.buf.bytes.len().saturating_add(take) > limit {
            return Err(ParseError::HeadTooLarge);
        }
        self.buf.append(&input[..take]);
        if !complete {
            return Ok(Step {
                consumed: take,
                event: Progress::NeedMore,
            });
        }
        let mut diags = diag::Diagnostics::new(self.profile);
        let (start, headers) =
            head::parse_head(self.kind, &self.buf.bytes, &self.limits, &mut diags)?;
        let framing = framing::resolve(
            &headers,
            &self.buf.bytes,
            self.kind,
            &start,
            self.response_context(),
            &mut diags,
        );
        let view = HeadView::new(start, headers, framing);
        let bytes = self.buf.bytes.split().freeze();
        self.buf.term = LEADING;
        self.state = body_state(view.framing());
        self.decoder.reset();
        Ok(Step {
            consumed: take,
            event: Progress::Head(RawMessage::new(bytes, view, self.kind, diags.into_vec())),
        })
    }
}

/// The body state a resolved framing starts in.
fn body_state(framing: &BodyFraming) -> State {
    match framing {
        BodyFraming::Ambiguous { resolved, .. } => body_state(resolved),
        BodyFraming::ContentLength(n) if *n > 0 => State::Fixed { remaining: *n },
        BodyFraming::Chunked => State::Chunked,
        BodyFraming::UntilClose => State::UntilClose,
        _ => State::Done,
    }
}

fn feed_until_close(input: &[u8]) -> Step {
    // Capped so that `consumed` never exceeds what the span can describe.
    let take = input.len().min(MAX_SPAN);
    if take == 0 {
        return Step {
            consumed: 0,
            event: Progress::NeedMore,
        };
    }
    let whole = Span::new(0, u32::try_from(take).unwrap_or(u32::MAX));
    Step {
        consumed: take,
        event: Progress::BodyData {
            raw: whole,
            data: whole,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::ParseLimits;

    #[test]
    fn capacity_never_exceeds_head_size() {
        for head_size in [1u32, 100, 4096, 4097, 10_000] {
            let cfg = ParserConfig::builder()
                .limits(ParseLimits::builder().head_size(head_size).build())
                .build();
            let mut p = Parser::request(&cfg);
            for _ in 0..head_size {
                p.feed(b"a").unwrap();
                assert!(p.buf.bytes.capacity() <= head_size as usize);
            }
            assert!(matches!(p.feed(b"a"), Err(ParseError::HeadTooLarge)));
            assert!(p.buf.bytes.capacity() <= head_size as usize);
        }
    }
}
