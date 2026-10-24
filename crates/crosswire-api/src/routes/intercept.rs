//! Intercept routes: inspect and configure the hold queue, and resolve held
//! requests by forwarding (unchanged or edited), dropping, or releasing all.

use axum::extract::{Path, State};
use axum::{Extension, Json};
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use bytes::Bytes;
use crosswire_proxy::{
    EditedBytes, HeldSummary, Intercept, InterceptError, InterceptRules, OnTimeout, Overflow,
};
use http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::app::AppState;
use crate::auth::{AuthCtx, Scope};
use crate::error::{ApiError, ApiResult};
use crate::routes::parse_ulid;

/// The intercept rules, over the wire.
#[derive(Serialize, Deserialize, ToSchema)]
pub struct RulesDto {
    /// Path extensions never held (without the dot).
    pub skip_extensions: Vec<String>,
    /// Skip out-of-scope requests.
    pub skip_out_of_scope: bool,
    /// `forward-newest` or `forward-oldest` on a full queue.
    pub overflow: String,
    /// `forward` or `respond-504` on a hold timeout.
    pub on_timeout: String,
}

impl From<InterceptRules> for RulesDto {
    fn from(r: InterceptRules) -> RulesDto {
        RulesDto {
            skip_extensions: r.skip_extensions,
            skip_out_of_scope: r.skip_out_of_scope,
            overflow: match r.overflow {
                Overflow::ForwardNewest => "forward-newest",
                Overflow::ForwardOldest => "forward-oldest",
                _ => "forward-newest",
            }
            .to_string(),
            on_timeout: match r.on_timeout {
                OnTimeout::Forward => "forward",
                OnTimeout::Respond504 => "respond-504",
                _ => "forward",
            }
            .to_string(),
        }
    }
}

impl RulesDto {
    /// Converts to the proxy rules, rejecting unknown enum spellings.
    fn into_rules(self) -> Result<InterceptRules, ApiError> {
        let overflow = match self.overflow.as_str() {
            "forward-newest" => Overflow::ForwardNewest,
            "forward-oldest" => Overflow::ForwardOldest,
            other => {
                return Err(ApiError::bad_request(
                    "bad_rules",
                    format!("unknown overflow: {other}"),
                ))
            }
        };
        let on_timeout = match self.on_timeout.as_str() {
            "forward" => OnTimeout::Forward,
            "respond-504" => OnTimeout::Respond504,
            other => {
                return Err(ApiError::bad_request(
                    "bad_rules",
                    format!("unknown on_timeout: {other}"),
                ))
            }
        };
        Ok(InterceptRules {
            skip_extensions: self.skip_extensions,
            skip_out_of_scope: self.skip_out_of_scope,
            overflow,
            on_timeout,
        })
    }
}

/// One held request.
#[derive(Serialize)]
pub struct HeldDto {
    /// The hold id.
    pub id: String,
    /// The addressed host.
    pub host: String,
    /// The request method.
    pub method: String,
    /// The request target.
    pub path: String,
    /// The buffered body size.
    pub size: usize,
    /// Whether the request may be edited.
    pub editable: bool,
}

impl From<HeldSummary> for HeldDto {
    fn from(h: HeldSummary) -> HeldDto {
        HeldDto {
            id: h.id.to_string(),
            host: h.host,
            method: h.method,
            path: h.path,
            size: h.size,
            editable: h.editable,
        }
    }
}

/// The intercept status.
#[derive(Serialize)]
pub struct InterceptStatus {
    /// Whether intercept is holding requests.
    pub enabled: bool,
    /// The current rules.
    pub rules: RulesDto,
    /// The held requests, oldest first.
    pub held: Vec<HeldDto>,
}

/// The `PUT /intercept/enabled` body.
#[derive(Deserialize, ToSchema)]
pub struct EnabledBody {
    /// Whether intercept is on.
    pub enabled: bool,
}

/// The `POST /intercept/held/{id}/forward` body.
#[derive(Deserialize, ToSchema)]
pub struct ForwardBody {
    /// A replacement request head, base64 (standard).
    pub head: Option<String>,
    /// A replacement request body, base64 (standard).
    pub body: Option<String>,
}

/// The current intercept status.
#[utoipa::path(
    get,
    path = "/api/v1/intercept",
    responses((status = 200, description = "Intercept status"))
)]
pub async fn get_intercept(
    State(state): State<AppState>,
    Extension(ctx): Extension<AuthCtx>,
) -> ApiResult<Json<InterceptStatus>> {
    ctx.scopes.require(Scope::Read)?;
    Ok(Json(InterceptStatus {
        enabled: state.intercept.enabled(),
        rules: state.intercept.rules().into(),
        held: state
            .intercept
            .held()
            .into_iter()
            .map(HeldDto::from)
            .collect(),
    }))
}

/// Turns intercept on or off.
#[utoipa::path(
    put,
    path = "/api/v1/intercept/enabled",
    responses((status = 200, description = "Intercept toggled"))
)]
pub async fn put_enabled(
    State(state): State<AppState>,
    Extension(ctx): Extension<AuthCtx>,
    Json(body): Json<EnabledBody>,
) -> ApiResult<StatusCode> {
    ctx.scopes.require(Scope::Admin)?;
    state.intercept.set_enabled(body.enabled);
    let _ = state.store.audit(
        &ctx.actor,
        "intercept.enabled",
        "",
        if body.enabled { "enabled" } else { "disabled" },
    );
    Ok(StatusCode::OK)
}

/// Replaces the intercept rules.
#[utoipa::path(
    put,
    path = "/api/v1/intercept/rules",
    responses(
        (status = 200, description = "Rules replaced"),
        (status = 400, description = "Unknown rule spelling")
    )
)]
pub async fn put_rules(
    State(state): State<AppState>,
    Extension(ctx): Extension<AuthCtx>,
    Json(body): Json<RulesDto>,
) -> ApiResult<StatusCode> {
    ctx.scopes.require(Scope::Admin)?;
    let rules = body.into_rules()?;
    state.intercept.set_rules(rules);
    let _ = state.store.audit(&ctx.actor, "intercept.rules", "", "ok");
    Ok(StatusCode::OK)
}

/// Maps an intercept error to an API error.
fn map_err(e: InterceptError) -> ApiError {
    match e {
        InterceptError::NotHeld => {
            ApiError::not_found("not_held", "no request is held with that id")
        }
        InterceptError::NotEditable => {
            ApiError::conflict("not_editable", "the request body was not buffered")
        }
        InterceptError::Edit(err) => ApiError::unprocessable(
            "edit_invalid",
            format!("edited request does not parse: {err}"),
        ),
        _ => ApiError::internal("intercept error"),
    }
}

/// Forwards a held request, unchanged or with edited bytes.
#[utoipa::path(
    post,
    path = "/api/v1/intercept/held/{id}/forward",
    responses(
        (status = 200, description = "Forwarded"),
        (status = 404, description = "Not held"),
        (status = 409, description = "Not editable"),
        (status = 422, description = "Edited bytes do not parse")
    )
)]
pub async fn forward_held(
    State(state): State<AppState>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
    Json(body): Json<ForwardBody>,
) -> ApiResult<StatusCode> {
    ctx.scopes.require(Scope::Send)?;
    let id = parse_ulid(&id)?;
    let edited = match (body.head, body.body) {
        (Some(head), body) => {
            let head = STANDARD
                .decode(head)
                .map_err(|_| ApiError::bad_request("bad_base64", "head is not valid base64"))?;
            let body = match body {
                Some(b) => Some(Bytes::from(STANDARD.decode(b).map_err(|_| {
                    ApiError::bad_request("bad_base64", "body is not valid base64")
                })?)),
                None => None,
            };
            Some(EditedBytes {
                head: Bytes::from(head),
                body,
            })
        }
        (None, Some(_)) => {
            return Err(ApiError::unprocessable(
                "head_required",
                "an edited body requires an edited head",
            ))
        }
        (None, None) => None,
    };
    state.intercept.forward(id, edited).map_err(map_err)?;
    Ok(StatusCode::OK)
}

/// Drops a held request, closing its client connection.
#[utoipa::path(
    post,
    path = "/api/v1/intercept/held/{id}/drop",
    responses(
        (status = 200, description = "Dropped"),
        (status = 404, description = "Not held")
    )
)]
pub async fn drop_held(
    State(state): State<AppState>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    ctx.scopes.require(Scope::Send)?;
    let id = parse_ulid(&id)?;
    // `Intercept::drop` shares a name with `Drop::drop`; call it fully
    // qualified to avoid the method-resolution trap.
    Intercept::drop(&state.intercept, id).map_err(map_err)?;
    Ok(StatusCode::OK)
}

/// Forwards every held request unchanged.
#[utoipa::path(
    post,
    path = "/api/v1/intercept/forward-all",
    responses((status = 200, description = "All forwarded"))
)]
pub async fn forward_all(
    State(state): State<AppState>,
    Extension(ctx): Extension<AuthCtx>,
) -> ApiResult<StatusCode> {
    ctx.scopes.require(Scope::Send)?;
    state.intercept.forward_all();
    Ok(StatusCode::OK)
}
