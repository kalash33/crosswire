//! Repeater routes: send a request and manage per-tab drafts.

use std::sync::PoisonError;

use crate::extract::{Json, Path};
use axum::extract::State;
use axum::Extension;
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use bytes::Bytes;
use crosswire_client::VerifyMode;
use crosswire_tools::{FollowRedirects, Mode, SendError, SendOutcome, SendRequest};
use http::StatusCode;
use serde::{Deserialize, Serialize};
use ulid::Ulid;
use utoipa::ToSchema;

use crate::app::AppState;
use crate::auth::{AuthCtx, Scope};
use crate::error::{ApiError, ApiResult};
use crate::routes::parse_ulid;

/// The largest draft payload accepted.
const MAX_DRAFT_BYTES: usize = 1024 * 1024;

/// The `POST /repeater/send` body (the `SendRequest` fields except `tab`).
#[derive(Deserialize, ToSchema)]
pub struct SendBody {
    /// A prior exchange this send derives from.
    pub source: Option<String>,
    /// The host to dial.
    pub host: String,
    /// The port to dial.
    pub port: u16,
    /// Connect over TLS.
    #[serde(default)]
    pub tls: bool,
    /// The SNI to send (defaults to `host`).
    pub sni: Option<String>,
    /// `web-pki` (default) or `accept-invalid`.
    pub verify: Option<String>,
    /// The raw request bytes, base64 (standard).
    pub raw: String,
    /// `assisted` or `raw` (default `raw`).
    #[serde(default)]
    pub mode: Option<String>,
    /// `off` (default), `in-scope` or `always`.
    pub follow: Option<String>,
    /// Reuse a kept-alive connection.
    #[serde(default)]
    pub reuse: bool,
    /// Allow an out-of-scope send when strict mode is off.
    #[serde(default)]
    pub allow_out_of_scope: bool,
}

/// One recorded edit, summarised.
#[derive(Serialize)]
pub struct ChangeDto {
    /// The edit operation (debug name).
    pub op: String,
    /// Why the edit was made (debug name).
    pub reason: String,
    /// The affected span's inclusive start offset.
    pub start: u32,
    /// The affected span's exclusive end offset.
    pub end: u32,
}

/// The outcome of a send.
#[derive(Serialize)]
pub struct SendOutcomeDto {
    /// The primary recorded exchange id.
    pub exchange_id: String,
    /// The scope verdict (wire name).
    pub scope_state: String,
    /// Edits assisted mode applied.
    pub changes: Vec<ChangeDto>,
    /// Notes about deliberately skipped edits.
    pub skipped: Vec<String>,
    /// Whether an invalid certificate chain was accepted.
    pub cert_override: bool,
    /// Time to first byte, milliseconds.
    pub ttfb_ms: u32,
    /// Total exchange time, milliseconds.
    pub total_ms: u32,
    /// Recorded redirect-hop exchange ids.
    pub redirects: Vec<String>,
}

impl From<SendOutcome> for SendOutcomeDto {
    fn from(o: SendOutcome) -> SendOutcomeDto {
        SendOutcomeDto {
            exchange_id: o.exchange_id.to_string(),
            scope_state: match serde_json::to_value(o.scope_state) {
                Ok(serde_json::Value::String(s)) => s,
                _ => String::new(),
            },
            changes: o
                .changes
                .iter()
                .map(|c| ChangeDto {
                    op: format!("{:?}", c.op()),
                    reason: format!("{:?}", c.reason()),
                    start: c.span().start(),
                    end: c.span().end(),
                })
                .collect(),
            skipped: o.skipped,
            cert_override: o.cert_override,
            ttfb_ms: o.ttfb_ms,
            total_ms: o.total_ms,
            redirects: o.redirects.iter().map(ToString::to_string).collect(),
        }
    }
}

/// A saved draft.
#[derive(Serialize)]
pub struct DraftDto {
    /// The tab id.
    pub tab_id: String,
    /// The draft payload.
    pub json: serde_json::Value,
    /// Last save time, unix milliseconds.
    pub updated_at: u64,
}

/// Sends a request through the repeater and records it.
#[utoipa::path(
    post,
    path = "/api/v1/repeater/send",
    responses(
        (status = 200, description = "The send outcome"),
        (status = 403, description = "Out of scope"),
        (status = 422, description = "The request could not be parsed"),
        (status = 502, description = "The upstream exchange failed")
    )
)]
pub async fn send_request(
    State(state): State<AppState>,
    Extension(ctx): Extension<AuthCtx>,
    Json(body): Json<SendBody>,
) -> ApiResult<Json<SendOutcomeDto>> {
    ctx.scopes.require(Scope::Send)?;
    let raw = STANDARD
        .decode(&body.raw)
        .map_err(|_| ApiError::bad_request("bad_base64", "raw is not valid base64"))?;
    let source = match body.source {
        Some(s) => Some(parse_ulid(&s)?),
        None => None,
    };
    let verify = match body.verify.as_deref() {
        None | Some("web-pki") => VerifyMode::WebPki,
        Some("accept-invalid") => VerifyMode::AcceptInvalid,
        Some(other) => {
            return Err(ApiError::bad_request(
                "bad_verify",
                format!("unknown verify mode: {other}"),
            ))
        }
    };
    let mode = match body.mode.as_deref() {
        None | Some("raw") => Mode::Raw,
        Some("assisted") => Mode::Assisted,
        Some(other) => {
            return Err(ApiError::bad_request(
                "bad_mode",
                format!("unknown mode: {other}"),
            ))
        }
    };
    let follow = match body.follow.as_deref() {
        None | Some("off") => FollowRedirects::Off,
        Some("in-scope") => FollowRedirects::InScope,
        Some("always") => FollowRedirects::Always,
        Some(other) => {
            return Err(ApiError::bad_request(
                "bad_follow",
                format!("unknown redirect policy: {other}"),
            ))
        }
    };
    let req = SendRequest {
        tab: Ulid::new(),
        source,
        host: body.host,
        port: body.port,
        tls: body.tls,
        sni: body.sni,
        verify,
        raw: Bytes::from(raw),
        mode,
        follow,
        reuse: body.reuse,
        allow_out_of_scope: body.allow_out_of_scope,
    };
    let outcome = state.repeater.send(req).await.map_err(map_send_err)?;
    Ok(Json(SendOutcomeDto::from(outcome)))
}

/// Maps a send error to an API error.
fn map_send_err(e: SendError) -> ApiError {
    match e {
        SendError::OutOfScope { host } => {
            ApiError::forbidden("out_of_scope", format!("out of scope: {host}"))
        }
        SendError::Parse(m) => ApiError::unprocessable("parse_error", m),
        SendError::Client(err) => ApiError::upstream(format!("upstream failed: {err}")),
        SendError::Sink(err) => ApiError::internal(format!("sink refused: {err}")),
        SendError::RedirectCap => {
            ApiError::unprocessable("redirect_cap", "redirects exceeded the ten-hop cap")
        }
        _ => ApiError::internal("send error"),
    }
}

/// Lists all saved repeater drafts.
#[utoipa::path(
    get,
    path = "/api/v1/repeater/tabs",
    responses((status = 200, description = "The saved drafts"))
)]
pub async fn list_tabs(
    State(state): State<AppState>,
    Extension(ctx): Extension<AuthCtx>,
) -> ApiResult<Json<Vec<DraftDto>>> {
    ctx.scopes.require(Scope::Read)?;
    let drafts = state
        .control
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .draft_list()
        .map_err(|e| ApiError::internal(format!("draft list failed: {e}")))?;
    let out = drafts
        .into_iter()
        .map(|d| DraftDto {
            tab_id: d.tab_id.to_string(),
            json: serde_json::from_str(&d.json).unwrap_or(serde_json::Value::Null),
            updated_at: d.updated_at,
        })
        .collect();
    Ok(Json(out))
}

/// Fetches one tab's saved draft.
#[utoipa::path(
    get,
    path = "/api/v1/repeater/tabs/{id}",
    responses(
        (status = 200, description = "The draft"),
        (status = 404, description = "No such draft")
    )
)]
pub async fn get_tab(
    State(state): State<AppState>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    ctx.scopes.require(Scope::Read)?;
    let id = parse_ulid(&id)?;
    let json = state
        .control
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .draft_get(id)
        .map_err(|e| ApiError::internal(format!("draft get failed: {e}")))?
        .ok_or_else(|| ApiError::not_found("not_found", "no such draft"))?;
    let value = serde_json::from_str(&json).unwrap_or(serde_json::Value::Null);
    Ok(Json(value))
}

/// Saves (or overwrites) a tab's draft (at most 1 MiB).
#[utoipa::path(
    put,
    path = "/api/v1/repeater/tabs/{id}",
    responses(
        (status = 200, description = "Saved"),
        (status = 413, description = "The draft exceeds 1 MiB")
    )
)]
pub async fn put_tab(
    State(state): State<AppState>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    Json(value): Json<serde_json::Value>,
) -> ApiResult<StatusCode> {
    ctx.scopes.require(Scope::Send)?;
    let id = parse_ulid(&id)?;
    let json = serde_json::to_string(&value)
        .map_err(|e| ApiError::internal(format!("encode failed: {e}")))?;
    if json.len() > MAX_DRAFT_BYTES {
        return Err(ApiError::too_large(
            "draft_too_large",
            "the draft exceeds 1 MiB",
        ));
    }
    state
        .control
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .draft_put(id, &json)
        .map_err(|e| ApiError::internal(format!("draft put failed: {e}")))?;
    Ok(StatusCode::OK)
}

/// Deletes a tab's draft.
#[utoipa::path(
    delete,
    path = "/api/v1/repeater/tabs/{id}",
    responses(
        (status = 200, description = "Deleted"),
        (status = 404, description = "No such draft")
    )
)]
pub async fn delete_tab(
    State(state): State<AppState>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    ctx.scopes.require(Scope::Send)?;
    let id = parse_ulid(&id)?;
    let removed = state
        .control
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .draft_delete(id)
        .map_err(|e| ApiError::internal(format!("draft delete failed: {e}")))?;
    if removed {
        Ok(StatusCode::OK)
    } else {
        Err(ApiError::not_found("not_found", "no such draft"))
    }
}
