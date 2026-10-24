//! Read and patch routes over recorded exchanges, markers and counts.

use std::sync::PoisonError;

use crate::extract::{Json, Path, Query};
use axum::extract::State;
use axum::response::IntoResponse;
use axum::Extension;
use crosswire_model::{Body, ExchangeState, Marker, NamespaceKind, ScopeState};
use crosswire_store::{BodySide, Counts, ExchangeFilter, ExchangeRow, Page};
use http::header::CONTENT_TYPE;
use http::StatusCode;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::app::AppState;
use crate::auth::{AuthCtx, Scope};
use crate::error::{ApiError, ApiResult};
use crate::routes::parse_ulid;

/// Query parameters for `GET /exchanges`.
#[derive(Debug, Default, Deserialize)]
pub struct ListQuery {
    /// Maximum rows to return (clamped to `1..=1000`).
    pub limit: Option<usize>,
    /// Keyset cursor: the previous page's `next_cursor`.
    pub after: Option<String>,
    /// Exact request method.
    pub method: Option<String>,
    /// Exact request host (ASCII case-insensitive).
    pub host: Option<String>,
    /// Substring of the request host.
    pub host_contains: Option<String>,
    /// Inclusive lower bound on the response status.
    pub status_min: Option<u16>,
    /// Inclusive upper bound on the response status.
    pub status_max: Option<u16>,
    /// The namespace (`history`, `repeater`, `intruder`).
    pub namespace: Option<String>,
    /// The recording state.
    pub state: Option<String>,
    /// The scope classification.
    pub scope_state: Option<String>,
    /// A tag the exchange must carry.
    pub tag: Option<String>,
}

/// Query parameters for `GET /markers`.
#[derive(Debug, Default, Deserialize)]
pub struct MarkerQuery {
    /// Keyset cursor.
    pub after: Option<String>,
    /// Maximum rows to return.
    pub limit: Option<usize>,
}

/// One history row.
#[derive(Serialize)]
pub struct ExchangeRowDto {
    /// The exchange id.
    pub id: String,
    /// Creation time, unix milliseconds.
    pub created_at: u64,
    /// Request method.
    pub method: Option<String>,
    /// Target host.
    pub host: Option<String>,
    /// Request target.
    pub path: Option<String>,
    /// Response status.
    pub status: Option<u16>,
    /// Response body length.
    pub resp_len: Option<u64>,
    /// Recording state (wire name).
    pub state: String,
    /// Tags.
    pub tags: Vec<String>,
}

impl From<ExchangeRow> for ExchangeRowDto {
    fn from(r: ExchangeRow) -> ExchangeRowDto {
        ExchangeRowDto {
            id: r.id.to_string(),
            created_at: r.created_at,
            method: r.method,
            host: r.host,
            path: r.path,
            status: r.status,
            resp_len: r.resp_len,
            state: wire(&r.state),
            tags: r.tags,
        }
    }
}

/// One page of results.
#[derive(Serialize)]
pub struct PageDto<T> {
    /// The rows, newest first.
    pub items: Vec<T>,
    /// Pass as `after` for the next page.
    pub next_cursor: Option<String>,
}

fn page_dto<T, U: From<T>>(page: Page<T>) -> PageDto<U> {
    PageDto {
        items: page.items.into_iter().map(U::from).collect(),
        next_cursor: page.next_cursor.map(|c| c.to_string()),
    }
}

/// Project totals.
#[derive(Serialize, ToSchema)]
pub struct CountsDto {
    /// All exchanges.
    pub exchanges: u64,
    /// Exchanges committed.
    pub committed: u64,
    /// Exchanges in error.
    pub error: u64,
    /// Markers.
    pub markers: u64,
    /// Bytes stored inline.
    pub bytes_inline: u64,
    /// Bytes stored in packs.
    pub bytes_packs: u64,
    /// Proxy-history exchanges.
    pub history: u64,
    /// Repeater exchanges.
    pub repeater: u64,
    /// Intruder exchanges.
    pub intruder: u64,
}

impl From<Counts> for CountsDto {
    fn from(c: Counts) -> CountsDto {
        CountsDto {
            exchanges: c.exchanges,
            committed: c.committed,
            error: c.error,
            markers: c.markers,
            bytes_inline: c.bytes_inline,
            bytes_packs: c.bytes_packs,
            history: c.by_namespace.history,
            repeater: c.by_namespace.repeater,
            intruder: c.by_namespace.intruder,
        }
    }
}

/// The `PATCH /exchanges/{id}` body.
#[derive(Deserialize, ToSchema)]
pub struct PatchBody {
    /// Replacement tags.
    pub tags: Option<Vec<String>>,
    /// Replacement notes.
    pub notes: Option<String>,
}

/// Serialises an enum to its wire name.
fn wire<T: Serialize>(v: &T) -> String {
    match serde_json::to_value(v) {
        Ok(serde_json::Value::String(s)) => s,
        _ => String::new(),
    }
}

/// Parses an enum from its wire name.
fn parse_wire<T: DeserializeOwned>(s: &str, field: &'static str) -> Result<T, ApiError> {
    serde_json::from_value(serde_json::Value::String(s.to_string()))
        .map_err(|_| ApiError::bad_request("bad_filter", format!("invalid {field} value")))
}

/// Lists recorded exchanges, newest first, with keyset paging and filters.
#[utoipa::path(
    get,
    path = "/api/v1/exchanges",
    params(
        ("limit" = Option<usize>, Query, description = "Maximum rows (1..=1000)"),
        ("after" = Option<String>, Query, description = "Keyset cursor"),
        ("method" = Option<String>, Query, description = "Exact request method"),
        ("host" = Option<String>, Query, description = "Exact request host"),
        ("host_contains" = Option<String>, Query, description = "Host substring"),
        ("status_min" = Option<u16>, Query, description = "Status lower bound"),
        ("status_max" = Option<u16>, Query, description = "Status upper bound"),
        ("namespace" = Option<String>, Query, description = "Namespace"),
        ("state" = Option<String>, Query, description = "Recording state"),
        ("scope_state" = Option<String>, Query, description = "Scope classification"),
        ("tag" = Option<String>, Query, description = "Required tag")
    ),
    responses((status = 200, description = "A page of exchange rows"))
)]
pub async fn list_exchanges(
    State(state): State<AppState>,
    Extension(ctx): Extension<AuthCtx>,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<PageDto<ExchangeRowDto>>> {
    ctx.scopes.require(Scope::Read)?;
    let after = match q.after {
        Some(s) => Some(parse_ulid(&s)?),
        None => None,
    };
    let status = match (q.status_min, q.status_max) {
        (None, None) => None,
        (min, max) => Some(min.unwrap_or(0)..=max.unwrap_or(u16::MAX)),
    };
    let filter = ExchangeFilter {
        method: q.method,
        host: q.host,
        host_contains: q.host_contains,
        status,
        namespace: q
            .namespace
            .as_deref()
            .map(|s| parse_wire::<NamespaceKind>(s, "namespace"))
            .transpose()?,
        state: q
            .state
            .as_deref()
            .map(|s| parse_wire::<ExchangeState>(s, "state"))
            .transpose()?,
        scope_state: q
            .scope_state
            .as_deref()
            .map(|s| parse_wire::<ScopeState>(s, "scope_state"))
            .transpose()?,
        tag: q.tag,
    };
    let limit = q.limit.unwrap_or(100);
    let page = state
        .store
        .list(&filter, after, limit)
        .map_err(|e| ApiError::internal(format!("list failed: {e}")))?;
    Ok(Json(page_dto::<ExchangeRow, ExchangeRowDto>(page)))
}

/// Fetches one exchange with its bodies rehydrated.
#[utoipa::path(
    get,
    path = "/api/v1/exchanges/{id}",
    responses(
        (status = 200, description = "The exchange"),
        (status = 404, description = "No such exchange")
    )
)]
pub async fn get_exchange(
    State(state): State<AppState>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
) -> ApiResult<Json<crosswire_model::Exchange>> {
    ctx.scopes.require(Scope::Read)?;
    let id = parse_ulid(&id)?;
    let exchange = state
        .store
        .get(id)
        .map_err(|e| ApiError::internal(format!("get failed: {e}")))?
        .ok_or_else(|| ApiError::not_found("not_found", "no such exchange"))?;
    Ok(Json(exchange))
}

/// Returns one body side as raw bytes, with its length and truncation
/// marker in headers.
#[utoipa::path(
    get,
    path = "/api/v1/exchanges/{id}/body/{side}",
    responses(
        (status = 200, description = "The body bytes", content_type = "application/octet-stream"),
        (status = 404, description = "No such body")
    )
)]
pub async fn get_body(
    State(state): State<AppState>,
    Extension(ctx): Extension<AuthCtx>,
    Path((id, side)): Path<(String, String)>,
) -> ApiResult<impl IntoResponse> {
    ctx.scopes.require(Scope::Read)?;
    let id = parse_ulid(&id)?;
    let side = match side.as_str() {
        "req" => BodySide::Req,
        "resp" => BodySide::Resp,
        _ => {
            return Err(ApiError::bad_request(
                "bad_side",
                "side must be req or resp",
            ))
        }
    };
    let bytes = state
        .store
        .get_body(id, side)
        .map_err(|e| ApiError::internal(format!("body read failed: {e}")))?
        .ok_or_else(|| ApiError::not_found("not_found", "no such body"))?;

    // The capture's truncation marker lives on the exchange's `Body`.
    let truncated_at = state.store.get(id).ok().flatten().and_then(|exchange| {
        let body = match side {
            BodySide::Req => Some(exchange.request().body().clone()),
            BodySide::Resp => exchange.response().map(|m| m.body().clone()),
        };
        body.and_then(|b| match b {
            Body::Inline { truncated_at, .. } | Body::Blob { truncated_at, .. } => truncated_at,
            _ => None,
        })
    });

    let mut headers = http::HeaderMap::new();
    headers.insert(CONTENT_TYPE, "application/octet-stream".parse().unwrap());
    headers.insert(
        "x-crosswire-length",
        bytes.len().to_string().parse().unwrap(),
    );
    if let Some(at) = truncated_at {
        headers.insert("x-crosswire-truncated-at", at.to_string().parse().unwrap());
    }
    Ok((StatusCode::OK, headers, bytes))
}

/// Lists markers, newest first.
#[utoipa::path(
    get,
    path = "/api/v1/markers",
    params(
        ("after" = Option<String>, Query, description = "Keyset cursor"),
        ("limit" = Option<usize>, Query, description = "Maximum rows (1..=1000)")
    ),
    responses((status = 200, description = "A page of markers"))
)]
pub async fn list_markers(
    State(state): State<AppState>,
    Extension(ctx): Extension<AuthCtx>,
    Query(q): Query<MarkerQuery>,
) -> ApiResult<Json<PageDto<Marker>>> {
    ctx.scopes.require(Scope::Read)?;
    let after = match q.after {
        Some(s) => Some(parse_ulid(&s)?),
        None => None,
    };
    let page = state
        .store
        .markers(after, q.limit.unwrap_or(100))
        .map_err(|e| ApiError::internal(format!("markers failed: {e}")))?;
    Ok(Json(PageDto {
        items: page.items,
        next_cursor: page.next_cursor.map(|c| c.to_string()),
    }))
}

/// Project totals.
#[utoipa::path(
    get,
    path = "/api/v1/counts",
    responses((status = 200, description = "Project totals", body = CountsDto))
)]
pub async fn get_counts(
    State(state): State<AppState>,
    Extension(ctx): Extension<AuthCtx>,
) -> ApiResult<Json<CountsDto>> {
    ctx.scopes.require(Scope::Read)?;
    let counts = state
        .store
        .counts()
        .map_err(|e| ApiError::internal(format!("counts failed: {e}")))?;
    Ok(Json(CountsDto::from(counts)))
}

/// Replaces an exchange's tags and/or notes.
#[utoipa::path(
    patch,
    path = "/api/v1/exchanges/{id}",
    responses(
        (status = 200, description = "Patched"),
        (status = 404, description = "No such exchange")
    )
)]
pub async fn patch_exchange(
    State(state): State<AppState>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    Json(body): Json<PatchBody>,
) -> ApiResult<StatusCode> {
    ctx.scopes.require(Scope::Send)?;
    let id = parse_ulid(&id)?;
    let control = state.control.lock().unwrap_or_else(PoisonError::into_inner);
    let mut found = true;
    if let Some(tags) = &body.tags {
        found &= control
            .exchange_set_tags(id, tags)
            .map_err(|e| ApiError::internal(format!("set tags failed: {e}")))?;
    }
    if let Some(notes) = &body.notes {
        found &= control
            .exchange_set_notes(id, notes)
            .map_err(|e| ApiError::internal(format!("set notes failed: {e}")))?;
    }
    if found {
        Ok(StatusCode::OK)
    } else {
        Err(ApiError::not_found("not_found", "no such exchange"))
    }
}
