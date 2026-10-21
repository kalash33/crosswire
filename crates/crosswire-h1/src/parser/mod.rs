//! The streaming, sans-IO message parser.

mod head;

use bytes::BytesMut;

use crate::config::ParserConfig;
use crate::error::ParseError;
use crate::limits::ParseLimits;
use crate::message::{BodyFraming, HeadView, Kind, RawMessage};
use crate::progress::{Progress, Step};

/// The bounded buffer a head is accumulated in. It never holds more than
/// `head_size` bytes: input is checked before it is copied in.
#[derive(Debug)]
struct HeadBuf {
    bytes: BytesMut,
    /// How much of the `\r\n\r\n` terminator the buffer currently ends with.
    term: u8,
}

impl HeadBuf {
    fn new() -> Self {
        HeadBuf {
            bytes: BytesMut::new(),
            term: 0,
        }
    }

    /// Returns how many bytes of `input` belong to the head (up to and
    /// including the terminator if it completes) and whether it completes.
    fn scan(&mut self, input: &[u8]) -> (usize, bool) {
        let mut term = self.term;
        for (i, &b) in input.iter().enumerate() {
            term = match (term, b) {
                (1, b'\n') => 2,
                (2, b'\r') => 3,
                (3, b'\n') => return (i + 1, true),
                (_, b'\r') => 1,
                _ => 0,
            };
        }
        self.term = term;
        (input.len(), false)
    }

    fn clear(&mut self) {
        self.bytes.clear();
        self.term = 0;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Accumulating the message head.
    Head,
    /// The message is complete. Body states arrive with framing (Task 5).
    Done,
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
            state: State::Head,
            buf: HeadBuf::new(),
        }
    }

    /// Feeds input and returns how much was consumed and what it produced.
    ///
    /// The `Head` step consumes exactly the bytes up to the end of the blank
    /// line. Once a message is `Done`, further input is left unconsumed.
    ///
    /// # Errors
    ///
    /// Returns a [`ParseError`] when a configured limit is exceeded.
    pub fn feed(&mut self, input: &[u8]) -> Result<Step, ParseError> {
        match self.state {
            State::Done => Ok(Step {
                consumed: 0,
                event: Progress::Done,
            }),
            State::Head => self.feed_head(input),
        }
    }

    /// Signals the end of the input stream.
    ///
    /// # Errors
    ///
    /// Returns [`ParseError::UnexpectedEof`] if a head is still incomplete.
    pub fn feed_eof(&mut self) -> Result<Step, ParseError> {
        match self.state {
            State::Done => Ok(Step {
                consumed: 0,
                event: Progress::Done,
            }),
            State::Head => Err(ParseError::UnexpectedEof),
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
        self.buf.bytes.extend_from_slice(&input[..take]);
        if !complete {
            return Ok(Step {
                consumed: take,
                event: Progress::NeedMore,
            });
        }
        let (start, headers) = head::parse_request_head(&self.buf.bytes, &self.limits)?;
        // Task 5: resolve framing from the headers.
        let view = HeadView::new(start, headers, BodyFraming::None);
        let bytes = self.buf.bytes.split().freeze();
        self.buf.term = 0;
        self.state = State::Done;
        Ok(Step {
            consumed: take,
            event: Progress::Head(RawMessage::new(
                bytes,
                view,
                self.kind,
                alloc::vec::Vec::new(),
            )),
        })
    }
}
