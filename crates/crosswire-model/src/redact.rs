//! Redaction of secrets in recorded messages.

use alloc::string::String;
use alloc::vec::Vec;
use bytes::Bytes;
use core::ops::Range;
use crosswire_h1::{Parser, ParserConfig, Progress, ResponseContext};
use serde::{Deserialize, Serialize};

use crate::message::{HeadSpans, HeaderSpanRecord, StartLineSpans};
use crate::{Fidelity, Head, Message};

const REDACTED: &[u8] = b"[REDACTED]";

/// Which header values and substrings to redact before a message is shared
/// or exported.
///
/// `header_names` are matched ASCII-case-insensitively against header names
/// (trailing spaces and tabs in the stored name are ignored); the whole value
/// is replaced with `[REDACTED]`.
///
/// `value_patterns` apply to every other header value. A pattern is a
/// literal prefix, matched ASCII-case-insensitively: the match is the prefix
/// plus the token that follows it, which runs until a space, tab, `;`, `,`,
/// `&`, quote or line break. The whole match is replaced with `[REDACTED]`
/// (so `Bearer abc.def` becomes `[REDACTED]`). The special pattern
/// [`RedactionPolicy::JWT_PATTERN`] (`@jwt`) matches the JWT shape: three
/// base64url segments separated by dots, the first starting `eyJ`.
///
/// Inline bodies are **not** redacted yet (a later task), blob bodies are
/// untouched, and recorded diffs are carried over unchanged. Only heads are
/// rewritten.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedactionPolicy {
    #[serde(default)]
    header_names: Vec<String>,
    #[serde(default)]
    value_patterns: Vec<String>,
}

impl RedactionPolicy {
    /// The built-in pattern for the JWT shape.
    pub const JWT_PATTERN: &'static str = "@jwt";

    /// Creates a policy.
    #[must_use]
    pub fn new(header_names: Vec<String>, value_patterns: Vec<String>) -> RedactionPolicy {
        RedactionPolicy {
            header_names,
            value_patterns,
        }
    }

    /// The default policy: common credential headers, `Bearer `,
    /// `access_token=`, `api_key=` and the JWT shape.
    #[must_use]
    pub fn default_policy() -> RedactionPolicy {
        let s = |v: &[&str]| v.iter().map(|x| String::from(*x)).collect();
        RedactionPolicy {
            header_names: s(&[
                "Authorization",
                "Proxy-Authorization",
                "Cookie",
                "Set-Cookie",
                "X-API-Key",
                "X-Auth-Token",
                "X-Amz-Security-Token",
            ]),
            value_patterns: s(&["Bearer ", "access_token=", "api_key=", Self::JWT_PATTERN]),
        }
    }

    /// Header names whose values are replaced whole.
    #[must_use]
    pub fn header_names(&self) -> &[String] {
        &self.header_names
    }

    /// Value patterns replaced wherever they match.
    #[must_use]
    pub fn value_patterns(&self) -> &[String] {
        &self.value_patterns
    }

    /// Returns a copy of `m` with secrets removed from its head.
    ///
    /// For an HTTP/1 head the replacements are applied to the original head
    /// bytes by span, from the end of the buffer backwards so earlier spans
    /// stay valid; the rebuilt bytes are re-parsed to get fresh spans and
    /// diagnostics, and the original framing record is kept. The result is
    /// labelled [`Fidelity::Normalized`]: it is no longer what was on the
    /// wire. A normalized head is redacted per pair. A tunnel head has
    /// nothing to redact and the message is returned unchanged. The body and
    /// diffs are not touched (see the type docs).
    #[must_use]
    pub fn redact_message(&self, m: &Message) -> Message {
        match m.head() {
            Head::Http1Raw {
                bytes,
                spans,
                parser_version,
            } => {
                let (new_bytes, reps) = self.rebuild_http1(bytes, spans);
                let (new_spans, diags) = reparse(&new_bytes, spans)
                    .map_or_else(|| (shift_spans(spans, &reps), None), |(s, d)| (s, Some(d)));
                m.clone()
                    .with_head(
                        Head::Http1Raw {
                            bytes: Bytes::from(new_bytes),
                            spans: new_spans,
                            parser_version: *parser_version,
                        },
                        diags,
                    )
                    .with_fidelity(Fidelity::Normalized)
            }
            Head::Normalized { map } => {
                let map = map
                    .iter()
                    .map(|(n, v)| {
                        if self.name_matches(n.as_bytes()) {
                            (n.clone(), REDACTED.to_vec())
                        } else {
                            let r = self.find_matches(v);
                            (n.clone(), replace_ranges(v, &r))
                        }
                    })
                    .collect();
                let h = Head::Normalized { map };
                m.clone()
                    .with_head(h, None)
                    .with_fidelity(Fidelity::Normalized)
            }
            _ => m.clone(),
        }
    }

    fn name_matches(&self, name: &[u8]) -> bool {
        let keep = name.iter().rposition(|&b| b != b' ' && b != b'\t');
        let name = &name[..keep.map_or(0, |i| i + 1)];
        !name.is_empty()
            && self
                .header_names
                .iter()
                .any(|h| h.as_bytes().eq_ignore_ascii_case(name))
    }

    /// Applies the policy to the head bytes; returns the new bytes and the
    /// replacements made (ranges in the original coordinates, sorted).
    fn rebuild_http1(
        &self,
        bytes: &[u8],
        spans: &HeadSpans,
    ) -> (Vec<u8>, Vec<(Range<usize>, usize)>) {
        let mut reps: Vec<Range<usize>> = Vec::new();
        for h in spans.headers() {
            let (Some(name), Some(value)) =
                (bytes.get(h.name().range()), bytes.get(h.value().range()))
            else {
                continue;
            };
            let vr = h.value().range();
            if self.name_matches(name) {
                if !value.is_empty() {
                    reps.push(vr);
                }
            } else {
                reps.extend(
                    self.find_matches(value)
                        .into_iter()
                        .map(|r| vr.start + r.start..vr.start + r.end),
                );
            }
        }
        reps.sort_by_key(|r| r.start);
        let out = replace_ranges(bytes, &reps);
        let deltas = reps
            .into_iter()
            .map(|r| {
                let len = r.len();
                (r, len)
            })
            .collect();
        (out, deltas)
    }

    /// Non-overlapping match ranges of all value patterns in `hay`, sorted.
    fn find_matches(&self, hay: &[u8]) -> Vec<Range<usize>> {
        let mut found: Vec<Range<usize>> = Vec::new();
        for p in &self.value_patterns {
            if p == Self::JWT_PATTERN {
                find_jwt(hay, &mut found);
            } else {
                find_prefixed(hay, p.as_bytes(), &mut found);
            }
        }
        found.sort_by_key(|r| (r.start, core::cmp::Reverse(r.end)));
        let mut merged: Vec<Range<usize>> = Vec::new();
        for r in found {
            match merged.last_mut() {
                Some(last) if r.start < last.end => last.end = last.end.max(r.end),
                _ => merged.push(r),
            }
        }
        merged
    }
}

fn is_token_delim(b: u8) -> bool {
    matches!(
        b,
        b' ' | b'\t' | b';' | b',' | b'&' | b'"' | b'\'' | b'\r' | b'\n'
    )
}

fn find_prefixed(hay: &[u8], prefix: &[u8], out: &mut Vec<Range<usize>>) {
    let n = prefix.len();
    if n == 0 || hay.len() < n {
        return;
    }
    let mut i = 0;
    while i + n <= hay.len() {
        if hay[i..i + n].eq_ignore_ascii_case(prefix) {
            let mut end = i + n;
            while end < hay.len() && !is_token_delim(hay[end]) {
                end += 1;
            }
            out.push(i..end);
            i = end;
        } else {
            i += 1;
        }
    }
}

fn is_b64url(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

fn find_jwt(hay: &[u8], out: &mut Vec<Range<usize>>) {
    let run = |from: usize| {
        let mut j = from;
        while j < hay.len() && is_b64url(hay[j]) {
            j += 1;
        }
        j
    };
    let mut i = 0;
    while i + 3 <= hay.len() {
        if &hay[i..i + 3] == b"eyJ" && (i == 0 || !is_b64url(hay[i - 1])) {
            let a = run(i);
            if hay.get(a) == Some(&b'.') {
                let b = run(a + 1);
                if b > a + 1 && hay.get(b) == Some(&b'.') {
                    let c = run(b + 1);
                    out.push(i..c);
                    i = c;
                    continue;
                }
            }
        }
        i += 1;
    }
}

/// Replaces each (sorted, non-overlapping) range of `src` with `[REDACTED]`,
/// working from the end so earlier offsets stay valid.
fn replace_ranges<R: core::borrow::Borrow<Range<usize>>>(src: &[u8], ranges: &[R]) -> Vec<u8> {
    let mut out = src.to_vec();
    for r in ranges.iter().rev() {
        let r = r.borrow();
        out.splice(r.start..r.end, REDACTED.iter().copied());
    }
    out
}

/// Re-parses rebuilt head bytes for fresh spans and diagnostics. The
/// original framing record is kept: it was resolved with request context a
/// bare re-parse does not have. `None` if the bytes no longer parse as one
/// complete head.
fn reparse(bytes: &[u8], old: &HeadSpans) -> Option<(HeadSpans, Vec<crosswire_core::Diagnostic>)> {
    let cfg = ParserConfig::default();
    let mut p = match old.start_line() {
        StartLineSpans::Response { .. } => {
            Parser::response(&cfg, ResponseContext::for_method(b"GET"))
        }
        _ => Parser::request(&cfg),
    };
    let step = p.feed(bytes).ok()?;
    let Progress::Head(raw) = step.event else {
        return None;
    };
    if step.consumed != bytes.len() {
        return None;
    }
    let fresh = HeadSpans::from(raw.head());
    Some((
        HeadSpans::new(
            *fresh.start_line(),
            fresh.headers().to_vec(),
            old.framing().clone(),
        ),
        raw.diagnostics().to_vec(),
    ))
}

/// Fallback when the rebuilt head does not re-parse: move every span by the
/// length change of the replacements before it. Replaced ranges are header
/// values, so a span start never falls inside one.
fn shift_spans(old: &HeadSpans, reps: &[(Range<usize>, usize)]) -> HeadSpans {
    let map = |o: u32| -> u32 {
        let o = o as usize;
        let mut out = o;
        for (r, len) in reps {
            if r.end <= o {
                out = out + REDACTED.len() - len;
            }
        }
        u32::try_from(out).unwrap_or(u32::MAX)
    };
    let sp = |s: crosswire_core::Span| crosswire_core::Span::new(map(s.start()), map(s.end()));
    let start = match old.start_line() {
        StartLineSpans::Response {
            version,
            status,
            reason,
        } => StartLineSpans::Response {
            version: sp(*version),
            status: sp(*status),
            reason: sp(*reason),
        },
        StartLineSpans::Request {
            method,
            target,
            version,
        } => StartLineSpans::Request {
            method: sp(*method),
            target: sp(*target),
            version: sp(*version),
        },
    };
    let headers = old
        .headers()
        .iter()
        .map(|h| HeaderSpanRecord::new(sp(h.name()), sp(h.value()), sp(h.line())))
        .collect();
    HeadSpans::new(start, headers, old.framing().clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixed_match_covers_token() {
        let p = RedactionPolicy::default_policy();
        assert_eq!(p.find_matches(b"x bearer abc;y"), alloc::vec![2..12]);
        assert_eq!(p.find_matches(b"a=1&api_key=Z9&b=2"), alloc::vec![4..14]);
        assert!(p.find_matches(b"nothing here").is_empty());
    }

    #[test]
    fn overlapping_matches_merge() {
        let p = RedactionPolicy::new(
            Vec::new(),
            alloc::vec!["Bearer ".into(), "Bearer eyJ".into()],
        );
        assert_eq!(p.find_matches(b"Bearer eyJabc"), alloc::vec![0..13]);
    }

    #[test]
    fn jwt_needs_three_segments() {
        let mut out = Vec::new();
        find_jwt(b"eyJa.eyJb", &mut out);
        assert!(out.is_empty());
        find_jwt(b"eyJa.b.c", &mut out);
        assert_eq!(out, alloc::vec![0..8]);
    }

    #[test]
    fn replace_ranges_from_the_end() {
        assert_eq!(
            replace_ranges(b"aXbYc", &[1..2, 3..4]),
            b"a[REDACTED]b[REDACTED]c"
        );
    }
}

#[cfg(test)]
mod shift_tests {
    use super::*;
    use crate::message::FramingRecord;
    use crosswire_core::Span;

    #[test]
    fn shift_moves_spans_after_a_replacement() {
        // "A: abcdef\r\nB: x\r\n": replace value 3..9 (6 bytes) by 10 bytes.
        let sp = Span::new;
        let old = HeadSpans::new(
            StartLineSpans::Request {
                method: sp(0, 0),
                target: sp(0, 0),
                version: sp(0, 0),
            },
            alloc::vec![
                HeaderSpanRecord::new(sp(0, 1), sp(3, 9), sp(0, 11)),
                HeaderSpanRecord::new(sp(11, 12), sp(14, 15), sp(11, 17)),
            ],
            FramingRecord::None,
        );
        let new = shift_spans(&old, &[(3..9, 6)]);
        assert_eq!(new.headers()[0].value(), sp(3, 13));
        assert_eq!(new.headers()[0].line(), sp(0, 15));
        assert_eq!(new.headers()[1].name(), sp(15, 16));
        assert_eq!(new.headers()[1].line(), sp(15, 21));
    }
}
