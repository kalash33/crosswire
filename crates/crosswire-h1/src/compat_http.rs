//! Lossy conversion of a [`RawMessage`] head into the `http` crate's types.
//!
//! See [`HttpConvertError`] for the conversion rules.

use alloc::borrow::Cow;
use alloc::vec::Vec;
use core::fmt;

use crosswire_core::Span;
use http::header::{HeaderName, HeaderValue};
use http::{HeaderMap, Method, StatusCode, Uri, Version};

use crate::message::{Kind, RawMessage, StartLine};

/// Why a [`RawMessage`] could not be converted to an `http` type.
///
/// Conversion exists for interoperability, not fidelity: `http` normalises and
/// rejects things this library deliberately preserves. The rules, which apply
/// to both [`http::Request`] and [`http::Response`] conversions:
///
/// - Header names are lower-cased by `http`; the original case is lost.
/// - An obs-fold value is joined by replacing each line terminator (`CRLF` or
///   a bare `LF`) plus the following run of spaces and tabs with one space.
/// - Trailing spaces and tabs in a header name (the `Host : x` shape) are
///   trimmed.
/// - More than one `Host` header (any case) is an error, not a merge.
/// - Bytes `http` rejects yield the matching error carrying the offending span.
/// - `HTTP/1.0` and `HTTP/1.1` are the only accepted versions.
/// - A response status must be three digits and within `http`'s range.
/// - The body is not converted (`()`); an inline body, if any, is ignored.
/// - Diagnostics and the exact wire bytes are not carried over.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum HttpConvertError {
    /// The message is a response (or request) but the other was requested.
    WrongKind,
    /// More than one `Host` header was present.
    DuplicateHost,
    /// A header name is not a valid `http` header name.
    InvalidHeaderName {
        /// The name bytes in the head.
        span: Span,
    },
    /// A header value is not a valid `http` header value.
    InvalidHeaderValue {
        /// The value bytes in the head.
        span: Span,
    },
    /// The method is not a valid `http` method.
    InvalidMethod {
        /// The method bytes in the head.
        span: Span,
    },
    /// The request target is not a valid `http` URI.
    InvalidUri {
        /// The target bytes in the head.
        span: Span,
    },
    /// The version is neither `HTTP/1.0` nor `HTTP/1.1`.
    InvalidVersion {
        /// The version bytes in the head.
        span: Span,
    },
    /// The status is not a valid three-digit `http` status code.
    InvalidStatus {
        /// The status bytes in the head.
        span: Span,
    },
}

impl fmt::Display for HttpConvertError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongKind => f.write_str("message kind does not match the requested type"),
            Self::DuplicateHost => f.write_str("duplicate Host header"),
            Self::InvalidHeaderName { span } => write!(f, "invalid header name at {span:?}"),
            Self::InvalidHeaderValue { span } => write!(f, "invalid header value at {span:?}"),
            Self::InvalidMethod { span } => write!(f, "invalid method at {span:?}"),
            Self::InvalidUri { span } => write!(f, "invalid request target at {span:?}"),
            Self::InvalidVersion { span } => write!(f, "unsupported HTTP version at {span:?}"),
            Self::InvalidStatus { span } => write!(f, "invalid status code at {span:?}"),
        }
    }
}

impl std::error::Error for HttpConvertError {}

fn slice(m: &RawMessage, span: Span) -> &[u8] {
    m.bytes().get(span.range()).unwrap_or(&[])
}

fn version(m: &RawMessage, span: Span) -> Result<Version, HttpConvertError> {
    match slice(m, span) {
        b"HTTP/1.1" => Ok(Version::HTTP_11),
        b"HTTP/1.0" => Ok(Version::HTTP_10),
        _ => Err(HttpConvertError::InvalidVersion { span }),
    }
}

/// Replaces each `CRLF`/`LF` plus following spaces/tabs with a single space.
fn unfold(v: &[u8]) -> Cow<'_, [u8]> {
    if !v.contains(&b'\n') {
        return Cow::Borrowed(v);
    }
    let mut out = Vec::with_capacity(v.len());
    let mut i = 0;
    while i < v.len() {
        match v[i] {
            b'\r' if v.get(i + 1) == Some(&b'\n') => {
                i += 2;
                i = skip_ows(v, i);
                out.push(b' ');
            }
            b'\n' => {
                i += 1;
                i = skip_ows(v, i);
                out.push(b' ');
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    Cow::Owned(out)
}

fn skip_ows(v: &[u8], mut i: usize) -> usize {
    while matches!(v.get(i), Some(b' ' | b'\t')) {
        i += 1;
    }
    i
}

fn headers(m: &RawMessage) -> Result<HeaderMap, HttpConvertError> {
    let mut map = HeaderMap::new();
    let mut seen_host = false;
    for h in m.head().headers() {
        let raw = slice(m, h.name());
        let keep = raw.iter().rposition(|&b| b != b' ' && b != b'\t');
        let name = &raw[..keep.map_or(0, |i| i + 1)];
        let name = HeaderName::from_bytes(name)
            .map_err(|_| HttpConvertError::InvalidHeaderName { span: h.name() })?;
        if name == http::header::HOST {
            if seen_host {
                return Err(HttpConvertError::DuplicateHost);
            }
            seen_host = true;
        }
        let value = HeaderValue::from_bytes(&unfold(slice(m, h.value())))
            .map_err(|_| HttpConvertError::InvalidHeaderValue { span: h.value() })?;
        map.append(name, value);
    }
    Ok(map)
}

/// Lossy conversion; see [`HttpConvertError`] for the rules.
impl TryFrom<&RawMessage> for http::Request<()> {
    type Error = HttpConvertError;

    fn try_from(m: &RawMessage) -> Result<Self, Self::Error> {
        let (
            StartLine::Request {
                method,
                target,
                version: ver,
            },
            Kind::Request,
        ) = (m.head().start(), m.kind())
        else {
            return Err(HttpConvertError::WrongKind);
        };
        let mut req = http::Request::new(());
        *req.method_mut() = Method::from_bytes(slice(m, *method))
            .map_err(|_| HttpConvertError::InvalidMethod { span: *method })?;
        *req.uri_mut() = Uri::try_from(slice(m, *target))
            .map_err(|_| HttpConvertError::InvalidUri { span: *target })?;
        *req.version_mut() = version(m, *ver)?;
        *req.headers_mut() = headers(m)?;
        Ok(req)
    }
}

/// Lossy conversion; see [`HttpConvertError`] for the rules.
impl TryFrom<&RawMessage> for http::Response<()> {
    type Error = HttpConvertError;

    fn try_from(m: &RawMessage) -> Result<Self, Self::Error> {
        let (
            StartLine::Response {
                version: ver,
                status,
                ..
            },
            Kind::Response,
        ) = (m.head().start(), m.kind())
        else {
            return Err(HttpConvertError::WrongKind);
        };
        let mut resp = http::Response::new(());
        *resp.version_mut() = version(m, *ver)?;
        *resp.status_mut() = StatusCode::from_bytes(slice(m, *status))
            .map_err(|_| HttpConvertError::InvalidStatus { span: *status })?;
        *resp.headers_mut() = headers(m)?;
        Ok(resp)
    }
}
