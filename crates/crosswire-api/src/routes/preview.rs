//! The sandboxed preview route on the isolated preview origin (spec §11.1).
//!
//! `GET /preview/{id}?token=...` serves one exchange's captured response body
//! for viewing in a sandboxed iframe on a DIFFERENT browser origin (`127.0.0.1`)
//! than the UI/API (`localhost`), so the UI's session cookie never attaches to a
//! previewed page. The route carries no cookie: the per-exchange HMAC capability
//! token in the query string (minted by `GET
//! /api/v1/exchanges/{id}/preview-token`) is the only credential, and it is
//! bound to the exact body bytes. The body is returned under a deny-all sandbox
//! CSP, with no `Set-Cookie` and no `Access-Control-*` header, so a captured
//! page can neither run script nor exfiltrate — even on its own origin.
//!
//! The origin itself is enforced by the gate ([`crate::auth::host_is_preview_origin`]):
//! a `/preview/` request on any other `Host`, `localhost` included, is a bare
//! `404`, so the body is never served off the isolated origin.

use std::time::SystemTime;

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use crosswire_model::{Head, Message};
use crosswire_store::BodySide;
use http::header::CONTENT_TYPE;
use http::{HeaderValue, StatusCode};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::app::AppState;
use crate::error::ApiError;
use crate::extract::{Path, Query};
use crate::routes::parse_ulid;

/// The deny-all sandbox CSP: no script, no network, no framing escape; only
/// `data:`/`blob:` images may load. A captured page rendered under this policy
/// cannot run script or exfiltrate, even on its own isolated origin.
const PREVIEW_CSP: &str = "sandbox; default-src 'none'; img-src data: blob:";

/// The banner prepended to a truncated HTML preview so the viewer knows the
/// page is incomplete. Plain markup: the deny-all CSP forbids inline styles.
const TRUNCATION_BANNER: &[u8] = b"<p>\xe2\x9a\xa0 Preview truncated.</p>\n";

/// Query parameters for `GET /preview/{id}`.
#[derive(Debug, Default, Deserialize)]
pub struct PreviewQuery {
    /// The capability token (base64url), carried in the URL, never a cookie.
    pub token: Option<String>,
}

/// Serves a captured response body on the isolated preview origin after
/// verifying the capability token against the exact body bytes.
///
/// The gate has already enforced the `127.0.0.1`/`[::1]` origin. On a verified
/// token the body is returned with its captured content-type, the deny-all
/// sandbox CSP, `Referrer-Policy: no-referrer` and `X-Content-Type-Options:
/// nosniff`, and never a `Set-Cookie` or `Access-Control-*` header. A forged,
/// expired or body-mismatched token is a minimal `403`; an unknown id or a
/// body-less exchange is a `404`.
pub async fn preview(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<PreviewQuery>,
) -> Response {
    match serve(&state, &id, q.token.as_deref()) {
        Ok(resp) => resp,
        Err(e) => e.into_response(),
    }
}

/// The fallible core of [`preview`].
fn serve(state: &AppState, id: &str, token: Option<&str>) -> Result<Response, ApiError> {
    // An unknown or malformed id reads as "no such resource" (404), not a 400
    // that would distinguish a bad id from a missing body.
    let ulid = parse_ulid(id).map_err(|_| not_found())?;

    // The body must exist before the token is weighed: a body-less exchange is a
    // 404, and the token is bound to the exact bytes we are about to hash.
    let body = state
        .store
        .get_body(ulid, BodySide::Resp)
        .map_err(|e| ApiError::internal(format!("body read failed: {e}")))?
        .ok_or_else(not_found)?;

    // The token is bound to the SHA-256 of these exact bytes (same digest the
    // mint route signed), so a changed or truncated body refuses the token.
    let body_hash: [u8; 32] = Sha256::digest(&body).into();
    let token = token.ok_or_else(forbidden)?;
    crate::preview_token::verify(
        &state.preview_key,
        ulid,
        &body_hash,
        token,
        SystemTime::now(),
    )
    .map_err(|_| forbidden())?;

    // The token is good. Reflect the captured content-type (defaulting to an
    // opaque octet-stream) so HTML renders as HTML and an image as an image; the
    // sandbox CSP and `nosniff` constrain what that rendering can do.
    let content_type = state
        .store
        .get(ulid)
        .ok()
        .flatten()
        .and_then(|ex| ex.response().and_then(response_content_type))
        .and_then(|ct| HeaderValue::from_str(&ct).ok())
        .unwrap_or_else(|| HeaderValue::from_static("application/octet-stream"));

    // Defence in depth: cap the previewed bytes at the proxy's capture limit
    // (bodies are already captured under it) and, when truncated, prepend a
    // visible banner for HTML so the viewer knows the page is incomplete.
    let cap =
        usize::try_from(crosswire_proxy::ProxyLimits::default().capture_cap).unwrap_or(usize::MAX);
    let is_html = content_type
        .to_str()
        .is_ok_and(|v| v.trim_start().to_ascii_lowercase().starts_with("text/html"));
    let bytes = cap_body(body, cap, is_html);

    let mut headers = http::HeaderMap::new();
    headers.insert(CONTENT_TYPE, content_type);
    headers.insert(
        "content-security-policy",
        HeaderValue::from_static(PREVIEW_CSP),
    );
    headers.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    Ok((StatusCode::OK, headers, bytes).into_response())
}

/// A minimal, non-leaking `404` (an unknown id or a body-less exchange).
fn not_found() -> ApiError {
    ApiError::not_found("not_found", "not found")
}

/// A minimal, non-leaking `403`. The reason (forged, expired, wrong body, or
/// absent token) is deliberately not distinguished.
fn forbidden() -> ApiError {
    ApiError::forbidden("forbidden", "the preview token is invalid")
}

/// Caps `body` to `cap` bytes. When truncated and `is_html`, prepend a short
/// banner so the viewer sees the preview is incomplete; otherwise the bytes are
/// just cut. Bodies are already captured under the proxy cap, so this is a
/// defence-in-depth backstop that almost never fires.
fn cap_body(mut body: Vec<u8>, cap: usize, is_html: bool) -> Vec<u8> {
    if body.len() <= cap {
        return body;
    }
    body.truncate(cap);
    if is_html {
        let mut out = Vec::with_capacity(TRUNCATION_BANNER.len() + body.len());
        out.extend_from_slice(TRUNCATION_BANNER);
        out.extend_from_slice(&body);
        out
    } else {
        body
    }
}

/// The response's `Content-Type` header value, read from the recorded head.
fn response_content_type(msg: &Message) -> Option<String> {
    header_value(msg, b"content-type")
}

/// The first value of header `name` (ASCII case-insensitive) in `msg`'s head,
/// trimmed, as a lossy UTF-8 string. Handles both a raw HTTP/1 head (sliced from
/// the stored bytes by span) and a translated header list.
fn header_value(msg: &Message, name: &[u8]) -> Option<String> {
    match msg.head() {
        Head::Http1Raw { bytes, spans, .. } => {
            for h in spans.headers() {
                let hn = bytes.get(h.name().range())?;
                if hn.eq_ignore_ascii_case(name) {
                    let hv = bytes.get(h.value().range())?;
                    return Some(String::from_utf8_lossy(hv).trim().to_string());
                }
            }
            None
        }
        Head::Normalized { map } => map
            .iter()
            .find(|(k, _)| k.as_bytes().eq_ignore_ascii_case(name))
            .map(|(_, v)| String::from_utf8_lossy(v).trim().to_string()),
        _ => None,
    }
}
