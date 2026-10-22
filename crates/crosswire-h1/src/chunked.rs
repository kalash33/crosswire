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
    /// The last-chunk line (`0...`) is complete and diagnosed; scanning the
    /// trailer block that follows it. Nothing is consumed until the block is
    /// complete, so the caller re-presents the input from the last-chunk line
    /// on. Offsets are relative to the start of that re-presented input:
    /// `lead` is where the trailer block starts, `verified` where the first
    /// line not yet known to be complete starts, and `searched` how far the
    /// search for its `\n` has already looked.
    Trailers {
        lead: usize,
        verified: usize,
        searched: usize,
    },
    Done,
}

/// The largest input prefix one call looks at, so that every offset (and
/// therefore `consumed`) fits a [`Span`].
const MAX_SPAN: usize = u32::MAX as usize;

/// A streaming decoder for a chunked message body, reused by [`Parser`](crate::Parser).
///
/// The decoder never buffers a partial token. The chunk-size line and the
/// trailer block are all-or-nothing: when the input does not yet contain one
/// up to and including its terminator, [`feed`](BodyDecoder::feed) returns
/// `(0, BodyEvent::NeedMore)` and the caller must re-present the unconsumed
/// bytes together with new data. Chunk data is consumed incrementally.
///
/// Between calls it keeps the phase, the remaining byte count of the current
/// chunk, whether an extension was already announced, the number of chunks
/// and bytes consumed so far, and, while a trailer block is incomplete, how
/// far into the re-presented bytes it has already verified complete lines.
/// That last offset lets a trailer block fed in small pieces be scanned in
/// linear time. It relies on the re-present contract: **re-presented bytes
/// must be identical to the ones presented before; the decoder does not
/// verify them.**
#[derive(Debug)]
pub struct BodyDecoder {
    limits: ParseLimits,
    profile: Profile,
    st: St,
    diags: Diagnostics,
    /// Body bytes consumed so far: the absolute offset of the current
    /// call's `input[0]` from the first body byte.
    pos: u64,
    /// Data chunks committed so far.
    chunks: u64,
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

fn is_ws(b: u8) -> bool {
    b == b' ' || b == b'\t'
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
            diags: Self::new_diags(limits, profile),
            pos: 0,
            chunks: 0,
        }
    }

    fn new_diags(limits: &ParseLimits, profile: Profile) -> Diagnostics {
        Diagnostics::with_cap(profile, usize::from(limits.max_body_diagnostics))
    }

    /// Diagnostics found so far in this body.
    ///
    /// Spans are absolute offsets from the first body byte (byte 0 is the
    /// first byte given to the first [`feed`](BodyDecoder::feed) call),
    /// saturating at `u32::MAX`. They do not depend on how the input was
    /// split across calls. At most
    /// [`ParseLimits::max_body_diagnostics`] are kept; the rest are counted by
    /// [`diagnostics_dropped`](BodyDecoder::diagnostics_dropped).
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        self.diags.as_slice()
    }

    /// How many body diagnostics were found but not kept because
    /// [`ParseLimits::max_body_diagnostics`] was reached.
    #[must_use]
    pub fn diagnostics_dropped(&self) -> usize {
        self.diags.dropped()
    }

    /// Whether the terminating chunk and trailers were fully read.
    pub(crate) fn is_done(&self) -> bool {
        matches!(self.st, St::Done)
    }

    /// Returns the decoder to its initial state, clearing diagnostics and
    /// the chunk and byte counters.
    pub(crate) fn reset(&mut self) {
        self.st = St::Size {
            ext_reported: false,
        };
        self.diags = Self::new_diags(&self.limits, self.profile);
        self.pos = 0;
        self.chunks = 0;
    }

    /// Records a diagnostic for the local range `start..end` of the current
    /// call's input, converted to absolute body offsets.
    fn diag(&mut self, code: DiagnosticCode, start: usize, end: usize) {
        let abs = |v: usize| {
            let v = self.pos.saturating_add(v as u64);
            u32::try_from(v).unwrap_or(u32::MAX)
        };
        let (s, e) = (abs(start), abs(end.max(start)));
        self.diags.push(code, Span::new(s, e));
    }

    /// Feeds input and returns how many bytes were consumed and what they
    /// produced. `consumed` never exceeds `input.len()` or `u32::MAX`, and
    /// every span points into `input`.
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
    /// Returns [`ParseError::InvalidChunkSize`] for a chunk size that is not
    /// hexadecimal or a malformed line break after chunk data,
    /// [`ParseError::LineTooLong`] for a chunk-size line over
    /// [`ParseLimits::max_chunk_size_line`], [`ParseError::TrailerTooLarge`]
    /// for a trailer block over [`ParseLimits::max_trailer_size`], and
    /// [`ParseError::TooManyChunks`] past [`ParseLimits::max_chunks`].
    pub fn feed(&mut self, input: &[u8]) -> Result<(usize, BodyEvent), ParseError> {
        if matches!(self.st, St::Done) {
            return Ok((0, BodyEvent::Done));
        }
        // Offsets past `u32::MAX` could not be described by a span; leave
        // such bytes for a later call.
        let input = &input[..input.len().min(MAX_SPAN)];
        if input.is_empty() {
            return Ok((0, BodyEvent::NeedMore));
        }
        let r = match self.st {
            St::Size { ext_reported } => self.feed_size(input, ext_reported),
            St::Data { remaining } => Ok(self.run_data(input, 0, remaining)),
            St::AfterData { .. } => self.feed_after_data(input),
            St::Trailers {
                lead,
                verified,
                searched,
            } => self.feed_trailers(input, lead, verified, searched),
            St::Done => Ok((0, BodyEvent::Done)),
        };
        if let Ok((n, _)) = &r {
            debug_assert!(*n <= input.len() && *n <= MAX_SPAN);
            self.pos = self.pos.saturating_add(*n as u64);
        }
        r
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
        let (raw_token, ext_at) = match content.iter().position(|&b| b == b';') {
            Some(i) => (&content[..i], Some(i)),
            None => (content, None),
        };
        // RFC 9112 section 7.1.1 allows BWS before `;`; whitespace before the
        // line break is tolerated the same way.
        let token_len = raw_token
            .iter()
            .rposition(|&b| !is_ws(b))
            .map_or(0, |i| i + 1);
        let token = &raw_token[..token_len];
        let size = parse_hex(token).ok_or(ParseError::InvalidChunkSize {
            span: span(0, raw_token.len()),
        })?;
        if size > 0 && self.chunks >= self.limits.max_chunks {
            return Err(ParseError::TooManyChunks);
        }
        // Diagnostics are only recorded once the line is complete and
        // committed, and only once per line even when it is re-presented
        // after a `ChunkExt` peek.
        if !ext_reported {
            if token.len() > 1 && token[0] == b'0' {
                self.diag(DiagnosticCode::ChunkSizeLeadingZeros, 0, token_len);
            }
            if token_len < raw_token.len() {
                self.diag(
                    DiagnosticCode::ChunkSizeWhitespace,
                    token_len,
                    raw_token.len(),
                );
            }
            if bare {
                self.diag(DiagnosticCode::BareLf, nl, nl.saturating_add(1));
            }
            if let Some(i) = ext_at {
                self.diag(DiagnosticCode::ChunkExtensionPresent, i + 1, content.len());
                self.st = St::Size { ext_reported: true };
                return Ok((0, BodyEvent::Ext(span(i + 1, content.len()))));
            }
        }
        if size == 0 {
            let lead = nl + 1;
            return self.feed_trailers(input, lead, lead, lead);
        }
        self.chunks += 1;
        self.st = St::Size {
            ext_reported: false,
        };
        Ok(self.run_data(input, nl + 1, size))
    }

    fn run_data(&mut self, input: &[u8], lead: usize, remaining: u64) -> (usize, BodyEvent) {
        let avail = input.len().saturating_sub(lead);
        // `input` is at most `MAX_SPAN` long, so `take`, and every offset
        // below, fits a span; `consumed` never outruns what spans describe.
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
        debug_assert!(raw_end <= MAX_SPAN);
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
                    self.diag(DiagnosticCode::BareLf, pos, pos + 1);
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

    /// Scans the trailer block that starts at `lead`, resuming at
    /// `verified` (the start of the first line not yet known to be complete)
    /// and `searched` (how far the `\n` search for that line got). All or
    /// nothing: `NeedMore` with nothing consumed until the final empty line
    /// is present.
    fn feed_trailers(
        &mut self,
        input: &[u8],
        lead: usize,
        verified: usize,
        searched: usize,
    ) -> Result<(usize, BodyEvent), ParseError> {
        let max = limit(self.limits.max_trailer_size);
        if verified > input.len() {
            // Fewer bytes than before were re-presented (a contract
            // violation); wait for them rather than misreading the input.
            return Ok((0, BodyEvent::NeedMore));
        }
        let mut pos = verified;
        let mut from = searched.clamp(verified, input.len());
        loop {
            let Some(i) = input[from..].iter().position(|&b| b == b'\n') else {
                // A lone `\r` may be the first half of the final empty line.
                let tail = &input[pos..];
                let tail = if tail == b"\r" { 0 } else { tail.len() };
                if (pos - lead).saturating_add(tail) > max {
                    return Err(ParseError::TrailerTooLarge);
                }
                self.st = St::Trailers {
                    lead,
                    verified: pos,
                    searched: input.len(),
                };
                return Ok((0, BodyEvent::NeedMore));
            };
            let nl = from + i;
            let content_end = if nl > pos && input[nl - 1] == b'\r' {
                nl - 1
            } else {
                nl
            };
            if content_end == pos {
                // The final empty line; it is not counted against the limit.
                return Ok(self.finish_trailers(input, lead, nl + 1));
            }
            if nl + 1 - lead > max {
                return Err(ParseError::TrailerTooLarge);
            }
            pos = nl + 1;
            from = pos;
        }
    }

    /// Completes the message with the verified trailer block
    /// `input[lead..end]` (`end` is just past the final empty line).
    fn finish_trailers(&mut self, input: &[u8], lead: usize, end: usize) -> (usize, BodyEvent) {
        self.st = St::Done;
        let block = &input[..end];
        let mut headers = Vec::new();
        let mut from = lead;
        while let Some(line) = find_line_end(block, from) {
            if line.len() == 0 {
                break;
            }
            if line.next - line.content_end == 1 {
                self.diag(DiagnosticCode::BareLf, line.content_end, line.next);
            }
            headers.push(split_header(block, line));
            from = line.next;
        }
        if headers.is_empty() {
            return (end, BodyEvent::Done);
        }
        let first = headers[0].line();
        self.diag(
            DiagnosticCode::TrailerPresent,
            first.start() as usize,
            first.end() as usize,
        );
        (end, BodyEvent::Trailers(headers))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_diagnostics_are_capped_without_growing() {
        let mut d = BodyDecoder::chunked(&ParseLimits::default());
        let mut body = Vec::new();
        for _ in 0..10_000 {
            body.extend_from_slice(b"01\nX\n");
        }
        let mut off = 0;
        while off < body.len() {
            let (n, _) = d.feed(&body[off..]).unwrap();
            assert!(n > 0);
            off += n;
        }
        assert_eq!(d.diagnostics().len(), 64);
        assert!(d.diagnostics_dropped() > 0);
        assert!(d.diags.as_slice().len() <= 64);
        assert!(
            d.diags.items_capacity() <= 64,
            "{}",
            d.diags.items_capacity()
        );
    }

    #[test]
    fn huge_remaining_does_not_overflow() {
        let mut d = BodyDecoder::chunked(&ParseLimits::default());
        d.st = St::Data {
            remaining: u64::MAX,
        };
        let (n, ev) = d.feed(b"abc").unwrap();
        assert_eq!(n, 3);
        assert_eq!(
            ev,
            BodyEvent::Data {
                raw: Span::new(0, 3),
                data: Span::new(0, 3)
            }
        );
        assert!(matches!(
            d.st,
            St::Data {
                remaining
            } if remaining == u64::MAX - 3
        ));
        let (n, _) = d.feed(b"de").unwrap();
        assert_eq!(n, 2);
        assert_eq!(d.pos, 5);
    }
}
