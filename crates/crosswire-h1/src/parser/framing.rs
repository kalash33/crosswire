//! Body framing resolution (RFC 9112 section 6.3), recording ambiguity.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use crosswire_core::{DiagnosticCode, Span};

use super::diag::Diagnostics;
use crate::message::{
    BodyFraming, HeaderSpan, Kind, Method, ResolutionRule, ResponseContext, StartLine,
};

fn is_ows(b: u8) -> bool {
    b == b' ' || b == b'\t'
}

/// Trims OWS from both ends of `span` within `buf`.
fn trim(buf: &[u8], span: Span) -> Span {
    let (mut s, mut e) = (span.start() as usize, span.end() as usize);
    e = e.min(buf.len());
    s = s.min(e);
    while s < e && is_ows(buf[s]) {
        s += 1;
    }
    while e > s && is_ows(buf[e - 1]) {
        e -= 1;
    }
    Span::new(s as u32, e as u32)
}

/// The headers named `name` (ASCII case-insensitive, trailing OWS in the
/// stored name ignored), in wire order.
fn named<'a>(
    headers: &'a [HeaderSpan],
    buf: &'a [u8],
    name: &'a str,
) -> impl Iterator<Item = &'a HeaderSpan> {
    headers.iter().filter(move |h| {
        buf.get(h.name().range()).is_some_and(|n| {
            let keep = n.iter().rposition(|&b| !is_ows(b)).map_or(0, |i| i + 1);
            n[..keep].eq_ignore_ascii_case(name.as_bytes())
        })
    })
}

/// Splits `span` at commas into trimmed pieces.
fn pieces(buf: &[u8], span: Span) -> Vec<Span> {
    let (s, e) = (span.start() as usize, (span.end() as usize).min(buf.len()));
    let mut out = Vec::new();
    let mut from = s;
    for i in s..e {
        if buf[i] == b',' {
            out.push(trim(buf, Span::new(from as u32, i as u32)));
            from = i + 1;
        }
    }
    out.push(trim(buf, Span::new(from.min(e) as u32, e as u32)));
    out
}

/// Parses one `Content-Length` piece: optional leading `+` (diagnosed once via
/// `plus_seen`), then ASCII digits that fit a `u64`.
fn parse_cl(buf: &[u8], piece: Span, plus_seen: &mut bool, d: &mut Diagnostics) -> Option<u64> {
    let text = buf.get(piece.range()).unwrap_or(&[]);
    let digits = match text.strip_prefix(b"+") {
        Some(rest) => {
            if !*plus_seen {
                *plus_seen = true;
                d.push(DiagnosticCode::ContentLengthLeadingPlus, piece);
            }
            rest
        }
        None => text,
    };
    let mut value: u64 = 0;
    let valid = !digits.is_empty()
        && digits.iter().all(|&b| {
            if !b.is_ascii_digit() {
                return false;
            }
            match value
                .checked_mul(10)
                .and_then(|v| v.checked_add(u64::from(b - b'0')))
            {
                Some(v) => {
                    value = v;
                    true
                }
                None => false,
            }
        });
    if valid {
        Some(value)
    } else {
        d.push(DiagnosticCode::ContentLengthNonNumeric, piece);
        None
    }
}

/// What a response/request with no usable length information is framed as.
fn fallback(kind: Kind) -> BodyFraming {
    match kind {
        Kind::Response => BodyFraming::UntilClose,
        _ => BodyFraming::None,
    }
}

/// At most this many distinct `Content-Length` values are kept as
/// candidates (the first ones, in wire order).
const MAX_CL_CANDIDATES: usize = 8;

/// The framing implied by the length headers alone, ignoring no-body rules.
fn from_headers(
    headers: &[HeaderSpan],
    buf: &[u8],
    kind: Kind,
    d: &mut Diagnostics,
) -> BodyFraming {
    // The first distinct valid Content-Length values, in wire order. Linear
    // in the number of values: the list never exceeds MAX_CL_CANDIDATES.
    let mut distinct: Vec<u64> = Vec::new();
    let mut total = 0usize;
    let mut invalid = false;
    let mut plus_seen = false;
    for h in named(headers, buf, "content-length") {
        for p in pieces(buf, h.value()) {
            total += 1;
            match parse_cl(buf, p, &mut plus_seen, d) {
                Some(v) => {
                    if distinct.len() < MAX_CL_CANDIDATES && !distinct.contains(&v) {
                        distinct.push(v);
                    }
                }
                None => invalid = true,
            }
        }
    }
    let first_cl = named(headers, buf, "content-length").next();
    if total > 1 {
        if let Some(h) = first_cl {
            d.push(DiagnosticCode::DuplicateContentLength, h.value());
        }
    }

    // Transfer-Encoding.
    let mut te_first: Option<Span> = None;
    let mut last_coding: Option<Span> = None;
    for h in named(headers, buf, "transfer-encoding") {
        te_first.get_or_insert(h.value());
        for p in pieces(buf, h.value()) {
            if p.start() != p.end() {
                last_coding = Some(p);
            }
        }
    }
    if let Some(te) = te_first {
        let chunked = last_coding
            .and_then(|c| buf.get(c.range()))
            .is_some_and(|c| c.eq_ignore_ascii_case(b"chunked"));
        let by_te = if chunked {
            BodyFraming::Chunked
        } else {
            d.push(
                DiagnosticCode::UnknownTransferCoding,
                last_coding.unwrap_or(te),
            );
            fallback(kind)
        };
        if first_cl.is_none() {
            return by_te;
        }
        d.push(DiagnosticCode::ConflictingFraming, te);
        let mut candidates = vec![by_te.clone()];
        if let Some(&v) = distinct.first() {
            candidates.push(BodyFraming::ContentLength(v));
        }
        return BodyFraming::Ambiguous {
            resolved: Box::new(by_te),
            candidates,
            rule: ResolutionRule::TransferEncodingWins,
        };
    }

    // RFC 9112 section 6.3 rule 5: an invalid Content-Length makes the
    // framing unrecoverable, even next to a valid one.
    if invalid {
        return BodyFraming::Ambiguous {
            resolved: Box::new(fallback(kind)),
            candidates: distinct
                .iter()
                .map(|&v| BodyFraming::ContentLength(v))
                .collect(),
            rule: ResolutionRule::InvalidContentLength,
        };
    }

    match distinct.as_slice() {
        [] => fallback(kind),
        [v] => BodyFraming::ContentLength(*v),
        [first, ..] => BodyFraming::Ambiguous {
            resolved: Box::new(BodyFraming::ContentLength(*first)),
            candidates: distinct
                .iter()
                .map(|&v| BodyFraming::ContentLength(v))
                .collect(),
            rule: ResolutionRule::FirstContentLength,
        },
    }
}

/// The framing the length headers imply, with an `Ambiguous` result reduced
/// to the framing it resolved to.
fn implied(headers: &[HeaderSpan], buf: &[u8], kind: Kind, d: &mut Diagnostics) -> BodyFraming {
    match from_headers(headers, buf, kind, d) {
        BodyFraming::Ambiguous { resolved, .. } => *resolved,
        other => other,
    }
}

/// Decides how the body of the message is delimited, appending any findings
/// to `d`. Pure over the head bytes: it never reads past `buf`, and
/// arithmetic on lengths is checked.
///
/// Responses (RFC 9110 sections 6.4.1, 8.6, 9.3.2, 15; RFC 9112 section 6.3):
/// - 101, or 2xx to CONNECT: `Upgrade`. With Content-Length or
///   Transfer-Encoding present (forbidden there) it is
///   `Ambiguous { resolved: Upgrade, rule: ConnectSuccess }` plus
///   `body_after_no_body_status`.
/// - other 1xx and 204: no body. With length headers (forbidden there) it is
///   `Ambiguous { resolved: None, rule: NoBodyStatus }` plus
///   `body_after_no_body_status`.
/// - a response to HEAD, and 304: no body. Length headers are allowed there
///   (they describe the representation), so the framing is plain `None`
///   with no `body_after_no_body_status`; malformed length headers are
///   still diagnosed.
pub(crate) fn resolve(
    headers: &[HeaderSpan],
    buf: &[u8],
    kind: Kind,
    start: &StartLine,
    ctx: Option<&ResponseContext>,
    d: &mut Diagnostics,
) -> BodyFraming {
    if let (Kind::Response, StartLine::Response { status, .. }) = (kind, start) {
        let code = buf.get(status.range()).and_then(super::parse_status);
        let is_connect = ctx.is_some_and(|c| c.is_connect);
        let is_head = ctx.is_some_and(|c| c.method == Method::Head);
        let upgrade =
            code == Some(101) || (is_connect && code.is_some_and(|c| (200..300).contains(&c)));
        let forbidden = code.is_some_and(|c| c < 200 || c == 204);
        if upgrade || forbidden {
            let (resolved, rule) = if upgrade {
                (BodyFraming::Upgrade, ResolutionRule::ConnectSuccess)
            } else {
                (BodyFraming::None, ResolutionRule::NoBodyStatus)
            };
            let marker = named(headers, buf, "content-length")
                .chain(named(headers, buf, "transfer-encoding"))
                .map(HeaderSpan::value)
                .min_by_key(|s| s.start());
            let Some(marker) = marker else {
                return resolved;
            };
            d.push(DiagnosticCode::BodyAfterNoBodyStatus, marker);
            let implied = implied(headers, buf, kind, d);
            return BodyFraming::Ambiguous {
                resolved: Box::new(resolved),
                candidates: vec![implied],
                rule,
            };
        }
        if is_head || code == Some(304) {
            // Run for the length headers' own diagnostics only.
            let _ = from_headers(headers, buf, kind, d);
            return BodyFraming::None;
        }
    }
    from_headers(headers, buf, kind, d)
}
