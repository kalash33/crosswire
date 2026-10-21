//! Incremental decoder for the chunked transfer coding.

use alloc::vec::Vec;

use crosswire_core::{Diagnostic, DiagnosticCode, Profile, Span};

use crate::error::ParseError;
use crate::limits::ParseLimits;
use crate::message::HeaderSpan;
use crate::parser::{find_line_end, split_header, Diagnostics};

/// What one [`BodyDecoder::feed`] call produced. All spans are offsets into
/// the `input` slice of that call.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum BodyEvent {
    /// More input is needed.
    NeedMore,
    /// The extension bytes after the `;` of a chunk-size line. This is a
    /// zero-consumption peek: feed the same input again to receive the
    /// chunk's [`BodyEvent::Data`].
    Ext(Span),
    /// Chunk bytes. `raw` is everything consumed by this call (the size line
    /// is only part of the first piece of a chunk, and the line break after
    /// the data is part of the piece that completes it); `data` is the
    /// payload subset.
    Data {
        /// The raw bytes as framed on the wire.
        raw: Span,
        /// The payload bytes.
        data: Span,
    },
    /// The trailer fields; the message is complete.
    Trailers(Vec<HeaderSpan>),
    /// The message is complete.
    Done,
}

#[derive(Debug, Clone, Copy)]
struct Chunk {
    size: u64,
    /// Bytes of the current input that belong to the size line.
    lead: usize,
}

#[derive(Debug)]
enum St {
    /// Expecting (or accumulating) a chunk-size line.
    Size,
    /// A size line was parsed and its extension announced.
    Announced(Chunk),
    /// Inside chunk data.
    Data {
        remaining: u64,
    },
    /// Expecting the line break after chunk data.
    AfterData {
        seen_cr: bool,
    },
    /// Inside the trailer section.
    Trailers,
    Done,
}

/// A streaming decoder for a chunked message body, reused by [`Parser`](crate::Parser).
///
/// Size lines and trailer blocks that arrive split across calls are buffered
/// (bounded by the configured limits). Spans of events produced from such
/// buffered bytes can only point into the current input for the part that
/// arrived in this call: a size line completed across calls reports its raw
/// span from the start of that call's input, and trailer spans for bytes
/// received in earlier calls point at the start of the trailer bytes of this
/// call. The `data` span is always exact for the bytes of this call.
#[derive(Debug)]
pub struct BodyDecoder {
    limits: ParseLimits,
    profile: Profile,
    st: St,
    /// A partial size line carried over from earlier calls.
    line: Vec<u8>,
    /// Trailer bytes accumulated so far.
    tbuf: Vec<u8>,
    /// Where the first incomplete trailer line starts in `tbuf`.
    tline: usize,
    diags: Diagnostics,
}

fn to_u32(v: usize) -> u32 {
    u32::try_from(v).unwrap_or(u32::MAX)
}

fn span(start: usize, end: usize) -> Span {
    Span::new(to_u32(start), to_u32(end.max(start)))
}

fn limit(v: u32) -> usize {
    usize::try_from(v).unwrap_or(usize::MAX)
}

impl BodyDecoder {
    /// Creates a decoder for a chunked body with the lenient profile.
    #[must_use]
    pub fn chunked(limits: &ParseLimits) -> Self {
        Self::with_profile(limits, Profile::default())
    }

    pub(crate) fn with_profile(limits: &ParseLimits, profile: Profile) -> Self {
        BodyDecoder {
            limits: *limits,
            profile,
            st: St::Size,
            line: Vec::new(),
            tbuf: Vec::new(),
            tline: 0,
            diags: Diagnostics::new(profile),
        }
    }

    /// Diagnostics found so far in this body.
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        self.diags.as_slice()
    }

    /// Whether the terminating chunk and trailers were fully read.
    pub(crate) fn is_done(&self) -> bool {
        matches!(self.st, St::Done)
    }

    /// Returns the decoder to its initial state, clearing diagnostics.
    pub(crate) fn reset(&mut self) {
        self.st = St::Size;
        self.line.clear();
        self.tbuf.clear();
        self.tline = 0;
        self.diags = Diagnostics::new(self.profile);
    }

    /// Feeds input and returns how many bytes were consumed and what they
    /// produced. `consumed` never exceeds `input.len()`.
    ///
    /// # Errors
    ///
    /// Returns [`ParseError::InvalidChunkSize`], [`ParseError::LineTooLong`]
    /// or [`ParseError::TrailerTooLarge`] for malformed or oversized framing.
    pub fn feed(&mut self, input: &[u8]) -> Result<(usize, BodyEvent), ParseError> {
        if matches!(self.st, St::Done) {
            return Ok((0, BodyEvent::Done));
        }
        if input.is_empty() {
            return Ok((0, BodyEvent::NeedMore));
        }
        match self.st {
            St::Size => self.feed_size(input),
            St::Announced(c) => self.proceed(input, c),
            St::Data { remaining } => Ok(self.run_data(input, 0, remaining)),
            St::AfterData { .. } => self.feed_after_data(input),
            St::Trailers => self.run_trailers(input, 0),
            St::Done => Ok((0, BodyEvent::Done)),
        }
    }

    fn feed_size(&mut self, input: &[u8]) -> Result<(usize, BodyEvent), ParseError> {
        let max = limit(self.limits.max_chunk_size_line);
        let Some(nl) = input.iter().position(|&b| b == b'\n') else {
            // +1 leaves room for the `\r` of a terminator split across calls.
            if self.line.len().saturating_add(input.len()) > max.saturating_add(1) {
                return Err(ParseError::LineTooLong);
            }
            self.line.extend_from_slice(input);
            return Ok((input.len(), BodyEvent::NeedMore));
        };
        let prefix = self.line.len();
        let combined: Vec<u8>;
        let mut content: &[u8] = if prefix == 0 {
            &input[..nl]
        } else {
            let mut v = core::mem::take(&mut self.line);
            v.extend_from_slice(&input[..nl]);
            combined = v;
            &combined
        };
        let bare = !content.ends_with(b"\r");
        if !bare {
            content = &content[..content.len() - 1];
        }
        if content.len() > max {
            return Err(ParseError::LineTooLong);
        }
        let map = |o: usize| o.saturating_sub(prefix);
        let (token, ext_at) = match content.iter().position(|&b| b == b';') {
            Some(i) => (&content[..i], Some(i)),
            None => (content, None),
        };
        let token_span = span(map(0), map(token.len()));
        let size = parse_hex(token).ok_or(ParseError::InvalidChunkSize { span: token_span })?;
        if token.len() > 1 && token[0] == b'0' {
            self.diags
                .push(DiagnosticCode::ChunkSizeLeadingZeros, token_span);
        }
        if bare {
            self.diags
                .push(DiagnosticCode::BareLf, span(nl, nl.saturating_add(1)));
        }
        let chunk = Chunk { size, lead: nl + 1 };
        self.line.clear();
        if let Some(i) = ext_at {
            let ext = span(map(i + 1), map(content.len()));
            self.diags.push(DiagnosticCode::ChunkExtensionPresent, ext);
            self.st = St::Announced(chunk);
            return Ok((0, BodyEvent::Ext(ext)));
        }
        self.proceed(input, chunk)
    }

    /// Continues after a size line: the data, or the trailers for size 0.
    fn proceed(&mut self, input: &[u8], c: Chunk) -> Result<(usize, BodyEvent), ParseError> {
        let lead = c.lead.min(input.len());
        if c.size == 0 {
            self.st = St::Trailers;
            return self.run_trailers(input, lead);
        }
        Ok(self.run_data(input, lead, c.size))
    }

    fn run_data(&mut self, input: &[u8], lead: usize, remaining: u64) -> (usize, BodyEvent) {
        let avail = input.len().saturating_sub(lead);
        let take = usize::try_from(remaining).map_or(avail, |r| r.min(avail));
        let end = lead + take;
        let left = remaining.saturating_sub(take as u64);
        let mut raw_end = end;
        if left == 0 {
            self.st = St::AfterData { seen_cr: false };
            // A bad byte is reported by the next call, after this data.
            raw_end = self.eat_line_break(input, end).0;
        } else {
            self.st = St::Data { remaining: left };
        }
        (
            raw_end,
            BodyEvent::Data {
                raw: span(0, raw_end),
                data: span(lead, end),
            },
        )
    }

    /// Consumes the line break after chunk data starting at `from`. Returns
    /// where it stopped and whether it stopped at a byte that is not part of a
    /// line break.
    fn eat_line_break(&mut self, input: &[u8], from: usize) -> (usize, bool) {
        let mut pos = from;
        while let (St::AfterData { seen_cr }, Some(&b)) = (&self.st, input.get(pos)) {
            let seen_cr = *seen_cr;
            match (seen_cr, b) {
                (false, b'\r') => self.st = St::AfterData { seen_cr: true },
                (false, b'\n') => {
                    self.diags.push(DiagnosticCode::BareLf, span(pos, pos + 1));
                    self.st = St::Size;
                }
                (true, b'\n') => self.st = St::Size,
                _ => return (pos, true),
            }
            pos += 1;
        }
        (pos, false)
    }

    fn feed_after_data(&mut self, input: &[u8]) -> Result<(usize, BodyEvent), ParseError> {
        let (n, bad) = self.eat_line_break(input, 0);
        if bad {
            return Err(ParseError::InvalidChunkSize {
                span: span(n, n + 1),
            });
        }
        Ok((
            n,
            BodyEvent::Data {
                raw: span(0, n),
                data: span(0, 0),
            },
        ))
    }

    fn run_trailers(
        &mut self,
        input: &[u8],
        lead: usize,
    ) -> Result<(usize, BodyEvent), ParseError> {
        // The final blank line is not counted against the limit.
        let max = limit(self.limits.max_trailer_size).saturating_add(2);
        let prev = self.tbuf.len();
        let start = lead.min(input.len());
        let mut pos = start;
        loop {
            let rest = &input[pos..];
            let Some(i) = rest.iter().position(|&b| b == b'\n') else {
                if self.tbuf.len().saturating_add(rest.len()) > max {
                    return Err(ParseError::TrailerTooLarge);
                }
                self.tbuf.extend_from_slice(rest);
                return Ok((input.len(), BodyEvent::NeedMore));
            };
            let piece = &rest[..=i];
            if self.tbuf.len().saturating_add(piece.len()) > max {
                return Err(ParseError::TrailerTooLarge);
            }
            self.tbuf.extend_from_slice(piece);
            pos += i + 1;
            let blank = find_line_end(&self.tbuf, self.tline).is_some_and(|l| l.len() == 0);
            if blank {
                break;
            }
            self.tline = self.tbuf.len();
        }
        let map = |o: usize| {
            if o >= prev {
                start + (o - prev)
            } else {
                start
            }
        };
        let tmap = |s: Span| span(map(s.start() as usize), map(s.end() as usize));
        let mut headers = Vec::new();
        let mut at = 0;
        while let Some(line) = find_line_end(&self.tbuf, at) {
            if line.len() == 0 {
                break;
            }
            if line.next - line.content_end == 1 {
                self.diags.push(
                    DiagnosticCode::BareLf,
                    span(map(line.content_end), map(line.next)),
                );
            }
            headers.push(map_header(split_header(&self.tbuf, line), tmap));
            at = line.next;
        }
        self.tbuf.clear();
        self.tline = 0;
        self.st = St::Done;
        if headers.is_empty() {
            return Ok((pos, BodyEvent::Done));
        }
        let first = headers[0].line();
        self.diags.push(DiagnosticCode::TrailerPresent, first);
        Ok((pos, BodyEvent::Trailers(headers)))
    }
}

fn map_header(h: HeaderSpan, f: impl Fn(Span) -> Span) -> HeaderSpan {
    HeaderSpan::new(f(h.name()), f(h.value()), f(h.line()))
}

/// Parses a non-empty run of hex digits that fits in a `u64`.
fn parse_hex(token: &[u8]) -> Option<u64> {
    if token.is_empty() {
        return None;
    }
    let mut v: u64 = 0;
    for &b in token {
        let d = char::from(b).to_digit(16)?;
        v = v.checked_mul(16)?.checked_add(u64::from(d))?;
    }
    Some(v)
}
