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

#[derive(Debug)]
enum St {
    /// Expecting a chunk-size line. `ext_reported` is set once the line's
    /// extension was announced with a zero-consumption [`BodyEvent::Ext`], so
    /// the re-presented line is not announced or diagnosed twice.
    Size {
        ext_reported: bool,
    },
    /// Inside chunk data.
    Data {
        remaining: u64,
    },
    /// Expecting the line break after chunk data.
    AfterData {
        seen_cr: bool,
    },
    Done,
}

/// A streaming decoder for a chunked message body, reused by [`Parser`](crate::Parser).
///
/// The decoder never buffers a partial token. The chunk-size line and the
/// trailer block are all-or-nothing: when the input does not yet contain one
/// up to and including its terminator, [`feed`](BodyDecoder::feed) returns
/// `(0, BodyEvent::NeedMore)` and the caller must re-present the unconsumed
/// bytes together with new data. Its only state between calls is the phase,
/// the remaining byte count of the current chunk and whether an extension was
/// already announced. Chunk data is consumed incrementally.
#[derive(Debug)]
pub struct BodyDecoder {
    limits: ParseLimits,
    profile: Profile,
    st: St,
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
            st: St::Size {
                ext_reported: false,
            },
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
        self.st = St::Size {
            ext_reported: false,
        };
        self.diags = Diagnostics::new(self.profile);
    }

    /// Feeds input and returns how many bytes were consumed and what they
    /// produced. `consumed` never exceeds `input.len()` and every span points
    /// into `input`.
    ///
    /// Size lines and trailer blocks are never buffered across calls. If
    /// `input` does not contain one in full, the result is
    /// `(0, BodyEvent::NeedMore)` and the caller must present the same bytes
    /// again, followed by new data, on the next call. An incomplete line or
    /// block that already exceeds its limit is an error instead. Chunk data is
    /// consumed incrementally, so a call may end in the middle of data.
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
            St::Size { ext_reported } => self.feed_size(input, ext_reported),
            St::Data { remaining } => Ok(self.run_data(input, 0, remaining)),
            St::AfterData { .. } => self.feed_after_data(input),
            St::Done => Ok((0, BodyEvent::Done)),
        }
    }

    fn feed_size(
        &mut self,
        input: &[u8],
        ext_reported: bool,
    ) -> Result<(usize, BodyEvent), ParseError> {
        let max = limit(self.limits.max_chunk_size_line);
        let Some(nl) = input.iter().position(|&b| b == b'\n') else {
            // A trailing `\r` may be the first half of the terminator.
            let held = input.len() - usize::from(input.ends_with(b"\r"));
            if held > max {
                return Err(ParseError::LineTooLong);
            }
            return Ok((0, BodyEvent::NeedMore));
        };
        let bare = nl == 0 || input[nl - 1] != b'\r';
        let content = &input[..if bare { nl } else { nl - 1 }];
        if content.len() > max {
            return Err(ParseError::LineTooLong);
        }
        let (token, ext_at) = match content.iter().position(|&b| b == b';') {
            Some(i) => (&content[..i], Some(i)),
            None => (content, None),
        };
        let token_span = span(0, token.len());
        let size = parse_hex(token).ok_or(ParseError::InvalidChunkSize { span: token_span })?;
        // Diagnostics are only recorded once the token is committed, so a
        // re-presented incomplete token never reports twice.
        let announce = ext_at.filter(|_| !ext_reported);
        let trailers = if size == 0 && announce.is_none() {
            match scan_trailers(input, nl + 1, limit(self.limits.max_trailer_size))? {
                Some(t) => Some(t),
                None => return Ok((0, BodyEvent::NeedMore)),
            }
        } else {
            None
        };
        if !ext_reported {
            if token.len() > 1 && token[0] == b'0' {
                self.diags
                    .push(DiagnosticCode::ChunkSizeLeadingZeros, token_span);
            }
            if bare {
                self.diags
                    .push(DiagnosticCode::BareLf, span(nl, nl.saturating_add(1)));
            }
        }
        if let Some(i) = announce {
            let ext = span(i + 1, content.len());
            self.diags.push(DiagnosticCode::ChunkExtensionPresent, ext);
            self.st = St::Size { ext_reported: true };
            return Ok((0, BodyEvent::Ext(ext)));
        }
        self.st = St::Size {
            ext_reported: false,
        };
        if let Some(t) = trailers {
            return Ok(self.finish_trailers(t));
        }
        Ok(self.run_data(input, nl + 1, size))
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
    /// line break. A `\r` at the very end of the input is consumed and
    /// remembered; the `\n` completing it is consumed by a later call, which
    /// reports an empty `Data` piece for it.
    fn eat_line_break(&mut self, input: &[u8], from: usize) -> (usize, bool) {
        let mut pos = from;
        while let (St::AfterData { seen_cr }, Some(&b)) = (&self.st, input.get(pos)) {
            let seen_cr = *seen_cr;
            match (seen_cr, b) {
                (false, b'\r') => self.st = St::AfterData { seen_cr: true },
                (false, b'\n') => {
                    self.diags.push(DiagnosticCode::BareLf, span(pos, pos + 1));
                    self.st = St::Size {
                        ext_reported: false,
                    };
                }
                (true, b'\n') => {
                    self.st = St::Size {
                        ext_reported: false,
                    }
                }
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

    /// Completes the message with a scanned trailer block.
    fn finish_trailers(&mut self, t: Trailers) -> (usize, BodyEvent) {
        self.st = St::Done;
        for s in t.bare_lf {
            self.diags.push(DiagnosticCode::BareLf, s);
        }
        if t.headers.is_empty() {
            return (t.end, BodyEvent::Done);
        }
        let first = t.headers[0].line();
        self.diags.push(DiagnosticCode::TrailerPresent, first);
        (t.end, BodyEvent::Trailers(t.headers))
    }
}

/// A fully present trailer block.
struct Trailers {
    /// Offset just past the final empty line.
    end: usize,
    headers: Vec<HeaderSpan>,
    bare_lf: Vec<Span>,
}

/// Scans the trailer block starting at `lead`, all or nothing: `None` until
/// the final empty line is present. Records nothing, so it can be repeated on
/// re-presented input.
fn scan_trailers(input: &[u8], lead: usize, max: usize) -> Result<Option<Trailers>, ParseError> {
    // The final empty line is not counted against the limit.
    let mut pos = lead;
    let mut headers = Vec::new();
    let mut bare_lf = Vec::new();
    loop {
        let Some(line) = find_line_end(input, pos) else {
            // A lone `\r` may be the first half of the final empty line.
            let tail = &input[pos..];
            let tail = if tail == b"\r" { 0 } else { tail.len() };
            if (pos - lead).saturating_add(tail) > max {
                return Err(ParseError::TrailerTooLarge);
            }
            return Ok(None);
        };
        if line.len() == 0 {
            return Ok(Some(Trailers {
                end: line.next,
                headers,
                bare_lf,
            }));
        }
        if line.next - lead > max {
            return Err(ParseError::TrailerTooLarge);
        }
        if line.next - line.content_end == 1 {
            bare_lf.push(span(line.content_end, line.next));
        }
        headers.push(split_header(input, line));
        pos = line.next;
    }
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
