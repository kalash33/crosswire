//! Byte edits on a parsed message, re-parsed so spans stay true.
//!
//! Every modification of a message (a repeater edit, an intercept edit, a
//! match-and-replace rule) goes through [`apply_edits`]. It splices the
//! edits into the message bytes, parses the result again with the caller's
//! [`ParserConfig`], and returns the new [`RawMessage`] together with one
//! [`Change`] per edit. The edited message is obtained from
//! [`Progress::Head`](crate::Progress::Head) like any other, so its spans
//! index its own frozen head bytes.

use alloc::vec::Vec;
use core::fmt;

use bytes::{Bytes, BytesMut};
use crosswire_core::{Change, ChangeOp, ChangeReason, Diagnostic, Span};

use crate::config::ParserConfig;
use crate::error::ParseError;
use crate::message::{Kind, Method, RawMessage, ResponseContext};
use crate::parser::{is_leading_ws, Parser};
use crate::progress::Progress;

/// One byte edit: replace the bytes under `span` with `replacement`.
///
/// `span` addresses the concatenation of the message's head bytes and its
/// inline body (`head ++ body`), in the coordinates of the message the edit
/// is applied to. An empty span is an insertion, an empty `replacement` a
/// deletion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    /// The bytes to replace, as offsets into `head ++ body`.
    pub span: Span,
    /// The bytes to put in their place.
    pub replacement: Bytes,
}

/// The result of [`apply_edits`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edited {
    /// The edited message, re-parsed. Everything after the new head's empty
    /// line is its inline body.
    pub message: RawMessage,
    /// One record per non-empty edit, in ascending order. Each span is in
    /// after-coordinates: it addresses the replacement inside the new
    /// `head ++ body`.
    pub changes: Vec<Change>,
    /// Diagnostics of the re-parsed head that the original head did not have
    /// at the same place. An original diagnostic counts as the same when its
    /// code matches and its span, moved by the edits before it, matches; a
    /// diagnostic whose span an edit touched is always reported.
    pub new_diagnostics: Vec<Diagnostic>,
}

/// Why [`apply_edits`] refused or failed.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum EditError {
    /// Two edits cover a common byte, or an insertion falls strictly inside
    /// another edit's span.
    Overlap,
    /// An edit reaches past the end of `head ++ body`, or the result would be
    /// longer than a [`Span`] can address.
    OutOfRange,
    /// An edit reaches past the head of a message without an inline body.
    BodyNotInline,
    /// The edited bytes no longer parse as a message.
    Reparse(ParseError),
}

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EditError::Overlap => f.write_str("edits overlap"),
            EditError::OutOfRange => f.write_str("edit out of range"),
            EditError::BodyNotInline => {
                f.write_str("body edit on a message without an inline body")
            }
            EditError::Reparse(e) => write!(f, "edited message does not parse: {e}"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for EditError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            EditError::Reparse(e) => Some(e),
            _ => None,
        }
    }
}

/// Applies `edits` to `msg` and re-parses the result.
///
/// Spans address `head ++ body`; an edit beyond the head needs an inline
/// body. The edits may be given in any order: they are sorted by start (an
/// insertion before an edit starting at the same offset; equal edits keep
/// their given order) and must not overlap. Each non-empty edit yields one
/// [`Change`] tagged with `reason`, whose span addresses the new bytes.
///
/// The new bytes are parsed with `cfg`, so the caller's profile and limits
/// apply. For a request `ctx` is ignored. For a response it is the request
/// the response answers, which decides the framing (`HEAD`, `CONNECT`); the
/// message does not remember it, so the caller passes it again. `None` means
/// a request other than `HEAD` or `CONNECT`.
///
/// The new head ends at the first empty line of the new bytes; everything
/// after it becomes the inline body, unchanged and unchecked against the new
/// framing (an edit may deliberately make them disagree). Without an inline
/// body on `msg` and with nothing after the new head, the result has none
/// either.
///
/// # Errors
///
/// - [`EditError::Overlap`] when two edits overlap.
/// - [`EditError::BodyNotInline`] when an edit reaches past the head and
///   `msg` has no inline body.
/// - [`EditError::OutOfRange`] when an edit reaches past `head ++ body`.
/// - [`EditError::Reparse`] when the new bytes do not parse, for example
///   [`ParseError::UnexpectedEof`] when an edit removed the empty line.
pub fn apply_edits(
    msg: &RawMessage,
    edits: &[Edit],
    cfg: &ParserConfig,
    ctx: Option<ResponseContext>,
    reason: ChangeReason,
) -> Result<Edited, EditError> {
    let head = msg.bytes();
    let body = msg.body();
    let total = head.len() + body.map_or(0, Bytes::len);

    let mut sorted: Vec<&Edit> = edits.iter().collect();
    sorted.sort_by_key(|e| (e.span.start(), e.span.end()));
    let mut prev_end = 0usize;
    for e in &sorted {
        let r = e.span.range();
        if r.start < prev_end {
            return Err(EditError::Overlap);
        }
        if r.end > head.len() && body.is_none() {
            return Err(EditError::BodyNotInline);
        }
        if r.end > total {
            return Err(EditError::OutOfRange);
        }
        prev_end = r.end;
    }

    let removed: usize = sorted.iter().map(|e| e.span.range().len()).sum();
    let added: usize = sorted.iter().map(|e| e.replacement.len()).sum();
    let new_len = total - removed + added;
    if u32::try_from(new_len).is_err() {
        return Err(EditError::OutOfRange);
    }

    let mut out = BytesMut::with_capacity(new_len);
    let mut changes = Vec::with_capacity(sorted.len());
    let mut at = 0usize;
    for e in &sorted {
        let r = e.span.range();
        copy_original(head, body, at..r.start, &mut out);
        let start = out.len();
        out.extend_from_slice(&e.replacement);
        at = r.end;
        let before = original_slice(head, body, r.clone());
        let op = match (r.is_empty(), e.replacement.is_empty()) {
            (true, true) => continue,
            (true, false) => ChangeOp::Insert,
            (false, true) => ChangeOp::Delete,
            (false, false) => ChangeOp::Replace,
        };
        changes.push(Change::new(
            op,
            span(start, out.len()),
            before,
            e.replacement.clone(),
            reason.clone(),
        ));
    }
    copy_original(head, body, at..total, &mut out);
    let new_bytes = out.freeze();

    let mut parser = match msg.kind() {
        Kind::Request => Parser::request(cfg),
        Kind::Response => Parser::response(
            cfg,
            ctx.unwrap_or(ResponseContext {
                method: Method::Other,
                is_connect: false,
            }),
        ),
    };
    let step = parser.feed(&new_bytes).map_err(EditError::Reparse)?;
    let mut message = match step.event {
        Progress::Head(m) => m,
        _ => {
            // The whole input was taken as an incomplete head.
            let err = parser.feed_eof().err().unwrap_or(ParseError::UnexpectedEof);
            return Err(EditError::Reparse(err));
        }
    };
    if body.is_some() || step.consumed < new_bytes.len() {
        message = message.with_inline_body(new_bytes.slice(step.consumed..));
    }

    let new_diagnostics = message
        .diagnostics()
        .iter()
        .filter(|d| {
            !msg.diagnostics()
                .iter()
                .any(|o| o.code() == d.code() && move_span(o.span(), &sorted) == Some(d.span()))
        })
        .copied()
        .collect();

    Ok(Edited {
        message,
        changes,
        new_diagnostics,
    })
}

/// The edit that makes `Content-Length` match the inline body's length.
///
/// The first `Content-Length` header (matched case-insensitively) gets the
/// body length as its value. When the message has an inline body that is
/// not empty but no `Content-Length` header, the edit inserts
/// `Content-Length: N\r\n` just before the head's empty line.
///
/// Returns `None` when there is no inline body, when the body is empty and
/// there is no header, when the value already equals the length (compared
/// as a decimal number, so `05` equals 5), and when `Transfer-Encoding` is
/// present: the length of a transfer-coded body is not its framing, so the
/// caller records the skip rather than rewriting the header.
#[must_use]
pub fn content_length_edit(msg: &RawMessage) -> Option<Edit> {
    let body = msg.body()?;
    let head = msg.bytes();
    let view = msg.head();
    if view.header(head, "transfer-encoding").is_some() {
        return None;
    }
    let len = body.len();
    let digits = Bytes::from(alloc::format!("{len}"));
    match view.header(head, "content-length") {
        Some(value) => {
            let current = head.get(value.range())?;
            if parse_decimal(current) == Some(len as u64) {
                return None;
            }
            Some(Edit {
                span: value,
                replacement: digits,
            })
        }
        None if len == 0 => None,
        None => {
            let term = if head.ends_with(b"\r\n") { 2 } else { 1 };
            let at = head.len().checked_sub(term)?;
            let mut line = Vec::with_capacity(20 + digits.len());
            line.extend_from_slice(b"Content-Length: ");
            line.extend_from_slice(&digits);
            line.extend_from_slice(b"\r\n");
            Some(Edit {
                span: span(at, at),
                replacement: Bytes::from(line),
            })
        }
    }
}

/// The edits that turn every bare LF line ending of the head into CRLF.
///
/// Each edit inserts one `\r` before a `\n` that does not already follow a
/// `\r`, including the empty line that ends the head. Whitespace skipped
/// before the start line is left alone. The head bytes are scanned directly,
/// which matches the parser's `bare_lf` diagnostics without depending on
/// them being present.
#[must_use]
pub fn crlf_edits(msg: &RawMessage) -> Vec<Edit> {
    let head = msg.bytes();
    let skip = head.iter().take_while(|&&b| is_leading_ws(b)).count();
    head.iter()
        .enumerate()
        .skip(skip)
        .filter(|&(i, &b)| b == b'\n' && head[i - 1] != b'\r')
        .map(|(i, _)| Edit {
            span: span(i, i),
            replacement: Bytes::from_static(b"\r"),
        })
        .collect()
}

/// A span from offsets already known to fit in `u32`.
fn span(start: usize, end: usize) -> Span {
    let to_u32 = |v: usize| u32::try_from(v).unwrap_or(u32::MAX);
    Span::new(to_u32(start), to_u32(end))
}

/// Appends the original bytes in `range` of `head ++ body` to `out`.
fn copy_original(
    head: &Bytes,
    body: Option<&Bytes>,
    range: core::ops::Range<usize>,
    out: &mut BytesMut,
) {
    let h = head.len();
    if range.start < h {
        out.extend_from_slice(&head[range.start..range.end.min(h)]);
    }
    if range.end > h {
        if let Some(b) = body {
            out.extend_from_slice(&b[range.start.max(h) - h..range.end - h]);
        }
    }
}

/// The original bytes in `range` of `head ++ body`, without copying when
/// the range lies within one of the two.
fn original_slice(head: &Bytes, body: Option<&Bytes>, range: core::ops::Range<usize>) -> Bytes {
    let h = head.len();
    if range.end <= h {
        return head.slice(range);
    }
    if range.start >= h {
        if let Some(b) = body {
            return b.slice(range.start - h..range.end - h);
        }
    }
    let mut out = BytesMut::with_capacity(range.len());
    copy_original(head, body, range, &mut out);
    out.freeze()
}

/// Where `s`, a span of the original head, lies after the sorted `edits`;
/// `None` when an edit touches it.
fn move_span(s: Span, edits: &[&Edit]) -> Option<Span> {
    let mut delta: i64 = 0;
    for e in edits {
        if e.span.end() <= s.start() {
            delta += e.replacement.len() as i64 - i64::from(e.span.len());
        } else if e.span.start() >= s.end() {
            break;
        } else {
            return None;
        }
    }
    let shift = |v: u32| u32::try_from(i64::from(v) + delta).ok();
    Some(Span::new(shift(s.start())?, shift(s.end())?))
}

/// An unsigned decimal made only of ASCII digits; `None` otherwise or on
/// overflow.
fn parse_decimal(text: &[u8]) -> Option<u64> {
    if text.is_empty() {
        return None;
    }
    text.iter().try_fold(0u64, |acc, &b| {
        if b.is_ascii_digit() {
            acc.checked_mul(10)?.checked_add(u64::from(b - b'0'))
        } else {
            None
        }
    })
}
