//! Pure functions that scan a buffered head and return spans into it.

use alloc::vec::Vec;

use crosswire_core::{DiagnosticCode, Span};

use super::diag::Diagnostics;

use crate::error::ParseError;
use crate::limits::ParseLimits;
use crate::message::{HeaderSpan, Kind, StartLine};

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

/// Finds the line starting at `from`. A line ends at the first `\n`; a `\r`
/// directly before it belongs to the terminator (`\r\n`), otherwise the
/// terminator is a bare `\n` (one byte, so `next - content_end == 1`).
pub(crate) fn find_line_end(buf: &[u8], from: usize) -> Option<Line> {
    let rest = buf.get(from..)?;
    let nl = rest.iter().position(|&b| b == b'\n')?;
    let content = if nl > 0 && rest[nl - 1] == b'\r' {
        nl - 1
    } else {
        nl
    };
    Some(Line {
        start: from,
        content_end: from + content,
        next: from + nl + 1,
    })
}

/// Whether the line ended with a bare `\n` rather than `\r\n`.
fn is_bare_lf(line: Line) -> bool {
    line.next - line.content_end == 1
}

/// Whether `b` may be skipped before the start line.
pub(crate) fn is_leading_ws(b: u8) -> bool {
    matches!(b, b'\r' | b'\n' | b' ' | b'\t')
}

fn is_ows(b: u8) -> bool {
    b == b' ' || b == b'\t'
}

/// RFC 9110 `tchar`.
fn is_tchar(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)
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

/// Splits a status line into version, status and reason. The version ends at
/// the first space and the status at the next one; the reason is the rest.
/// Missing parts become empty spans at the end of the line.
pub(crate) fn split_status_line(buf: &[u8], line: Line) -> StartLine {
    let first_space = |from: usize| {
        buf[from..line.content_end]
            .iter()
            .position(|&b| b == b' ')
            .map(|i| from + i)
    };
    let (version_end, status_start) = match first_space(line.start) {
        Some(i) => (i, i + 1),
        None => (line.content_end, line.content_end),
    };
    let (status_end, reason_start) = match first_space(status_start) {
        Some(i) => (i, i + 1),
        None => (line.content_end, line.content_end),
    };
    StartLine::Response {
        version: span(line.start, version_end),
        status: span(status_start, status_end),
        reason: span(reason_start, line.content_end),
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
    let after = &text[colon + 1..];
    let lead = after.iter().take_while(|&&b| is_ows(b)).count();
    let trail = after[lead..]
        .iter()
        .rev()
        .take_while(|&&b| is_ows(b))
        .count();
    let v_start = line.start + colon + 1 + lead;
    let v_end = line.content_end - trail;
    HeaderSpan::new(name, span(v_start, v_end), whole)
}

/// Reports a lone `\r` inside the line's content and a bare `\n` terminator.
fn diagnose_line(buf: &[u8], line: Line, d: &mut Diagnostics) {
    for (i, &b) in buf[line.start..line.content_end].iter().enumerate() {
        if b == b'\r' {
            d.push(
                DiagnosticCode::BareCr,
                span(line.start + i, line.start + i + 1),
            );
        }
    }
    if is_bare_lf(line) {
        d.push(DiagnosticCode::BareLf, span(line.content_end, line.next));
    }
}

/// Whether `target` starts with `scheme://`.
fn is_absolute_form(target: &[u8]) -> bool {
    let scheme = target
        .iter()
        .take_while(|&&b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
        .count();
    scheme > 0 && target[0].is_ascii_alphabetic() && target[scheme..].starts_with(b"://")
}

fn diagnose_request_line(buf: &[u8], line: Line, start: &StartLine, d: &mut Diagnostics) {
    let StartLine::Request {
        target, version, ..
    } = start
    else {
        return;
    };
    let whole = span(line.start, line.content_end);
    let spaces = buf[line.start..line.content_end]
        .iter()
        .filter(|&&b| b == b' ')
        .count();
    if spaces < 2 {
        d.push(DiagnosticCode::Http09Request, whole);
    } else if !matches!(&buf[version.range()], b"HTTP/1.0" | b"HTTP/1.1") {
        d.push(DiagnosticCode::VersionUnusual, *version);
    }
    if is_absolute_form(&buf[target.range()]) {
        d.push(DiagnosticCode::AbsoluteFormTarget, *target);
    }
}

/// Reports problems in a status line.
///
/// The catalogue has no `malformed_status_line` code in v0.1, so
/// `version_unusual` is the catch-all: it is emitted when the version is not
/// exactly `HTTP/1.0` or `HTTP/1.1`, and also when the status is missing or is
/// not exactly three ASCII digits (so `HeadView::status_code` would be `None`).
/// The code is emitted at most once, on the version span.
fn diagnose_response_line(buf: &[u8], start: &StartLine, d: &mut Diagnostics) {
    let StartLine::Response {
        version, status, ..
    } = start
    else {
        return;
    };
    let version_ok = matches!(&buf[version.range()], b"HTTP/1.0" | b"HTTP/1.1");
    if !version_ok || parse_status(&buf[status.range()]).is_none() {
        d.push(DiagnosticCode::VersionUnusual, *version);
    }
}

/// Parses exactly three ASCII digits.
pub(crate) fn parse_status(text: &[u8]) -> Option<u16> {
    match text {
        [a, b, c] if text.iter().all(u8::is_ascii_digit) => {
            Some(u16::from(a - b'0') * 100 + u16::from(b - b'0') * 10 + u16::from(c - b'0'))
        }
        _ => None,
    }
}

/// Reports everything odd about one header line (not a fold continuation).
fn diagnose_header(buf: &[u8], h: &HeaderSpan, d: &mut Diagnostics) {
    let has_colon = buf.get(h.name().end() as usize) == Some(&b':');
    let name_start = h.name().start() as usize;
    let name = &buf[h.name().range()];
    let trailing = name.iter().rev().take_while(|&&b| is_ows(b)).count();
    let trimmed = &name[..name.len() - trailing];
    let trimmed_span = span(name_start, name_start + trimmed.len());
    if trailing > 0 {
        d.push(
            DiagnosticCode::WhitespaceBeforeColon,
            span(name_start + trimmed.len(), name_start + name.len()),
        );
        d.push(DiagnosticCode::TrailingSpaceInName, h.name());
    }
    if !has_colon {
        d.push(DiagnosticCode::InvalidTokenCharInName, h.name());
        return;
    }
    if trimmed.is_empty() || !trimmed.iter().all(|&b| is_tchar(b)) {
        d.push(DiagnosticCode::InvalidTokenCharInName, trimmed_span);
    } else if !is_canonical_case(trimmed) {
        d.push(DiagnosticCode::NonCanonicalHeaderCase, trimmed_span);
    }
    diagnose_value(buf, h.value(), d);
}

/// Reports the first control byte in `value`. CR is reported as `bare_cr`, so
/// it is not repeated here.
fn diagnose_value(buf: &[u8], value: Span, d: &mut Diagnostics) {
    let start = value.start() as usize;
    let is_ctl = |b: u8| (b < 0x20 && b != b'\t' && b != b'\r') || b == 0x7f;
    if let Some(i) = buf[value.range()].iter().position(|&b| is_ctl(b)) {
        d.push(
            DiagnosticCode::InvalidCharInValue,
            span(start + i, start + i + 1),
        );
    }
}

/// Whether `name` is in `Title-Case`: each letter after the start or a `-`
/// uppercase, every other letter lowercase.
fn is_canonical_case(name: &[u8]) -> bool {
    let mut upper_next = true;
    for &b in name {
        if b.is_ascii_alphabetic() && b.is_ascii_uppercase() != upper_next {
            return false;
        }
        upper_next = b == b'-';
    }
    true
}

/// Extends the previous header over an obsolete-fold continuation line.
fn fold_into(buf: &[u8], prev: &HeaderSpan, line: Line) -> HeaderSpan {
    let text = &buf[line.start..line.content_end];
    let trail = text.iter().rev().take_while(|&&b| is_ows(b)).count();
    let value_end = line.content_end - trail;
    let value_start = if prev.value().is_empty() {
        line.start + text.iter().take_while(|&&b| is_ows(b)).count()
    } else {
        prev.value().start() as usize
    };
    HeaderSpan::new(
        prev.name(),
        span(value_start, value_end.max(value_start)),
        span(prev.line().start() as usize, line.next),
    )
}

/// Parses a complete head (ending in a blank line) into its start line,
/// header spans and diagnostics, enforcing `max_line` and `max_headers`. The
/// start line is split and diagnosed according to `kind`; the header scanner is
/// shared. Malformed input is annotated, never rejected.
pub(crate) fn parse_head(
    kind: Kind,
    buf: &[u8],
    limits: &ParseLimits,
    d: &mut Diagnostics,
) -> Result<(StartLine, Vec<HeaderSpan>), ParseError> {
    let max_line = usize::try_from(limits.max_line).unwrap_or(usize::MAX);
    let skip = buf.iter().take_while(|&&b| is_leading_ws(b)).count();
    if skip > 0 {
        d.push(
            DiagnosticCode::LeadingWhitespaceBeforeStartLine,
            span(0, skip),
        );
    }
    let first = find_line_end(buf, skip).ok_or(ParseError::NoTerminator)?;
    if first.len() > max_line {
        return Err(ParseError::LineTooLong);
    }
    let start = match kind {
        Kind::Response => split_status_line(buf, first),
        _ => split_request_line(buf, first),
    };
    diagnose_line(buf, first, d);
    match kind {
        Kind::Response => diagnose_response_line(buf, &start, d),
        _ => diagnose_request_line(buf, first, &start, d),
    }
    let mut headers: Vec<HeaderSpan> = Vec::new();
    let mut pos = first.next;
    let end = loop {
        let line = find_line_end(buf, pos).ok_or(ParseError::NoTerminator)?;
        diagnose_line(buf, line, d);
        if line.len() == 0 {
            break pos;
        }
        if line.len() > max_line {
            return Err(ParseError::LineTooLong);
        }
        let continuation = is_ows(buf[line.start]);
        if let (true, Some(prev)) = (continuation, headers.last_mut()) {
            d.push(DiagnosticCode::ObsFold, span(line.start, line.content_end));
            *prev = fold_into(buf, prev, line);
            diagnose_value(buf, span(line.start, line.content_end), d);
        } else {
            if headers.len() >= usize::from(limits.max_headers) {
                return Err(ParseError::TooManyHeaders);
            }
            let h = split_header(buf, line);
            diagnose_header(buf, &h, d);
            headers.push(h);
        }
        pos = line.next;
    };
    diagnose_fields(kind, buf, &headers, end, d);
    Ok((start, headers))
}

/// Cross-header checks: `Host` presence and uniqueness (requests only), `Expect`.
fn diagnose_fields(
    kind: Kind,
    buf: &[u8],
    headers: &[HeaderSpan],
    end: usize,
    d: &mut Diagnostics,
) {
    let request = kind != Kind::Response;
    let mut hosts = 0usize;
    for h in headers {
        if buf.get(h.name().end() as usize) != Some(&b':') {
            continue;
        }
        let name = trim_ows_end(&buf[h.name().range()]);
        if name.eq_ignore_ascii_case(b"host") {
            hosts += 1;
            if request && hosts > 1 {
                d.push(DiagnosticCode::DuplicateHost, h.name());
            }
        } else if name.eq_ignore_ascii_case(b"expect")
            && buf[h.value().range()].eq_ignore_ascii_case(b"100-continue")
        {
            d.push(DiagnosticCode::Expect100Continue, h.value());
        }
    }
    if request && hosts == 0 {
        d.push(DiagnosticCode::MissingHost, span(end, end));
    }
}

fn trim_ows_end(b: &[u8]) -> &[u8] {
    let n = b.iter().rev().take_while(|&&c| is_ows(c)).count();
    &b[..b.len() - n]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_end_reports_content_and_terminator() {
        let l = find_line_end(b"ab\r\ncd", 0).unwrap();
        assert_eq!((l.start, l.content_end, l.next), (0, 2, 4));
        // A bare LF terminates a line on its own.
        let l = find_line_end(b"ab\ncd\r\n", 0).unwrap();
        assert_eq!((l.content_end, l.next), (2, 3));
        let l = find_line_end(b"ab\ncd\r\n", 3).unwrap();
        assert_eq!((l.content_end, l.next), (5, 7));
        let l = find_line_end(b"\r\n", 0).unwrap();
        assert_eq!((l.content_end, l.next), (0, 2));
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
