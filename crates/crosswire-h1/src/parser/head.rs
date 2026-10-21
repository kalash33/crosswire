//! Pure functions that scan a buffered head and return spans into it.

use alloc::vec::Vec;

use crosswire_core::Span;

use crate::error::ParseError;
use crate::limits::ParseLimits;
use crate::message::{HeaderSpan, StartLine};

/// Builds a span from buffer offsets. Offsets never exceed `head_size`, which is a `u32`.
fn span(start: usize, end: usize) -> Span {
    let to_u32 = |v: usize| u32::try_from(v).unwrap_or(u32::MAX);
    Span::new(to_u32(start), to_u32(end))
}

/// One line found in a buffer: its content and where the next line starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Line {
    /// Offset of the first content byte.
    pub(crate) start: usize,
    /// Offset one past the last content byte (the terminator starts here).
    pub(crate) content_end: usize,
    /// Offset one past the terminator, where the next line starts.
    pub(crate) next: usize,
}

impl Line {
    /// The content bytes' length, excluding the terminator.
    pub(crate) fn len(&self) -> usize {
        self.content_end - self.start
    }
}

/// Finds the line starting at `from`. Only `\r\n` terminates a line for now;
/// the content and terminator are reported separately so lenient terminators
/// can be added without changing callers.
pub(crate) fn find_line_end(buf: &[u8], from: usize) -> Option<Line> {
    let rest = buf.get(from..)?;
    let mut i = 0;
    while let Some(off) = rest.get(i..)?.iter().position(|&b| b == b'\n') {
        let nl = i + off;
        if nl > 0 && rest[nl - 1] == b'\r' {
            return Some(Line {
                start: from,
                content_end: from + nl - 1,
                next: from + nl + 1,
            });
        }
        i = nl + 1;
    }
    None
}

/// Splits a request line into method, target and version at single spaces.
/// Missing parts become empty spans at the end of the line.
pub(crate) fn split_request_line(buf: &[u8], line: Line) -> StartLine {
    let text = &buf[line.start..line.content_end];
    let sp1 = text.iter().position(|&b| b == b' ');
    let (method_end, target_start) = match sp1 {
        Some(i) => (line.start + i, line.start + i + 1),
        None => (line.content_end, line.content_end),
    };
    let target_rest = &buf[target_start..line.content_end];
    let (target_end, version_start) = match target_rest.iter().position(|&b| b == b' ') {
        Some(i) => (target_start + i, target_start + i + 1),
        None => (line.content_end, line.content_end),
    };
    StartLine::Request {
        method: span(line.start, method_end),
        target: span(target_start, target_end),
        version: span(version_start, line.content_end),
    }
}

/// Splits a header line at the first colon and trims optional whitespace
/// around the value. A line without a colon yields an empty value.
pub(crate) fn split_header(buf: &[u8], line: Line) -> HeaderSpan {
    let text = &buf[line.start..line.content_end];
    let whole = span(line.start, line.next);
    let Some(colon) = text.iter().position(|&b| b == b':') else {
        return HeaderSpan::new(
            span(line.start, line.content_end),
            span(line.content_end, line.content_end),
            whole,
        );
    };
    let name = span(line.start, line.start + colon);
    let is_ows = |b: &u8| *b == b' ' || *b == b'\t';
    let after = &text[colon + 1..];
    let lead = after.iter().take_while(|b| is_ows(b)).count();
    let trail = after[lead..].iter().rev().take_while(|b| is_ows(b)).count();
    let v_start = line.start + colon + 1 + lead;
    let v_end = line.content_end - trail;
    HeaderSpan::new(name, span(v_start, v_end), whole)
}

/// Parses a complete request head (ending in a blank line) into its start
/// line and header spans, enforcing `max_line` and `max_headers`.
pub(crate) fn parse_request_head(
    buf: &[u8],
    limits: &ParseLimits,
) -> Result<(StartLine, Vec<HeaderSpan>), ParseError> {
    let max_line = usize::try_from(limits.max_line).unwrap_or(usize::MAX);
    let first = find_line_end(buf, 0).ok_or(ParseError::NoTerminator)?;
    if first.len() > max_line {
        return Err(ParseError::LineTooLong);
    }
    let start = split_request_line(buf, first);
    let mut headers = Vec::new();
    let mut pos = first.next;
    loop {
        let line = find_line_end(buf, pos).ok_or(ParseError::NoTerminator)?;
        if line.len() == 0 {
            return Ok((start, headers));
        }
        if line.len() > max_line {
            return Err(ParseError::LineTooLong);
        }
        if headers.len() >= usize::from(limits.max_headers) {
            return Err(ParseError::TooManyHeaders);
        }
        headers.push(split_header(buf, line));
        pos = line.next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_end_reports_content_and_terminator() {
        let l = find_line_end(b"ab\r\ncd", 0).unwrap();
        assert_eq!((l.start, l.content_end, l.next), (0, 2, 4));
        assert_eq!(find_line_end(b"ab\ncd\r\n", 0).unwrap().content_end, 5);
        assert!(find_line_end(b"ab\r", 0).is_none());
        assert!(find_line_end(b"ab", 5).is_none());
    }

    #[test]
    fn header_value_is_trimmed() {
        let buf = b"X:  a b \t\r\n";
        let l = find_line_end(buf, 0).unwrap();
        let h = split_header(buf, l);
        assert_eq!(&buf[h.value().range()], b"a b");
        assert_eq!(h.line(), Span::new(0, 11));
    }

    #[test]
    fn malformed_lines_do_not_panic() {
        let buf = b"GET\r\nnocolon\r\n";
        let l = find_line_end(buf, 0).unwrap();
        let StartLine::Request {
            target, version, ..
        } = split_request_line(buf, l)
        else {
            panic!()
        };
        assert!(target.is_empty() && version.is_empty());
        let l = find_line_end(buf, l.next).unwrap();
        assert!(split_header(buf, l).value().is_empty());
    }
}
