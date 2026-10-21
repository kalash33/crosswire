//! The streaming, sans-IO message parser.

mod diag;
mod head;

use bytes::BytesMut;

use crate::config::ParserConfig;
use crate::error::ParseError;
use crate::limits::ParseLimits;
use crate::message::{BodyFraming, HeadView, Kind, RawMessage};
use crate::progress::{Progress, Step};
use crosswire_core::Profile;

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

#[derive(Debug, Clone, PartialEq, Eq)]
enum State {
    /// Accumulating the message head.
    Head,
    /// The message is complete. Body states arrive with framing (Task 5).
    Done,
    /// A fatal error occurred; it is returned until [`Parser::reset`].
    Failed(ParseError),
}

/// An incremental HTTP/1 message parser.
///
/// Feed it bytes with [`feed`](Parser::feed) until it returns
/// [`Progress::NeedMore`]. After [`Progress::Done`], the unconsumed tail of the
/// input is the next pipelined message; call [`reset`](Parser::reset) to parse it.
#[derive(Debug)]
pub struct Parser {
    kind: Kind,
    limits: ParseLimits,
    profile: Profile,
    state: State,
    buf: HeadBuf,
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
        }
    }

    /// Feeds input and returns how much was consumed and what it produced.
    ///
    /// The `Head` step consumes exactly the bytes up to the end of the blank
    /// line. Once a message is `Done`, further input is left unconsumed. After
    /// an error, every later call returns the same error until
    /// [`reset`](Parser::reset).
    ///
    /// # Errors
    ///
    /// Returns a [`ParseError`] when a configured limit is exceeded.
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
        };
        if let Err(e) = &result {
            self.state = State::Failed(e.clone());
        }
        result
    }

    /// Signals the end of the input stream.
    ///
    /// # Errors
    ///
    /// Returns [`ParseError::UnexpectedEof`] if a head is still incomplete, or
    /// the earlier error if the parser has already failed.
    pub fn feed_eof(&mut self) -> Result<Step, ParseError> {
        match &self.state {
            State::Done => Ok(Step {
                consumed: 0,
                event: Progress::Done,
            }),
            State::Failed(e) => Err(e.clone()),
            State::Head => {
                self.state = State::Failed(ParseError::UnexpectedEof);
                Err(ParseError::UnexpectedEof)
            }
        }
    }

    /// Discards all state and starts parsing a new message.
    pub fn reset(&mut self) {
        self.state = State::Head;
        self.buf.clear();
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
        let (start, headers) = head::parse_request_head(&self.buf.bytes, &self.limits, &mut diags)?;
        // Task 5: resolve framing from the headers.
        let view = HeadView::new(start, headers, BodyFraming::None);
        let bytes = self.buf.bytes.split().freeze();
        self.buf.term = LEADING;
        self.state = State::Done;
        Ok(Step {
            consumed: take,
            event: Progress::Head(RawMessage::new(bytes, view, self.kind, diags.into_vec())),
        })
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
