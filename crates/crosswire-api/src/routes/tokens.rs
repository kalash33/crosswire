//! Token routes: list token metadata, mint a token (shown once), and revoke.

use std::sync::PoisonError;

use axum::extract::{Path, State};
use axum::{Extension, Json};
use crosswire_store::{TokenRecord, TokenScope};
use http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::app::AppState;
use crate::auth::{AuthCtx, Scope};
use crate::error::{ApiError, ApiResult};
use crate::routes::parse_ulid;

/// An API token's metadata (never its secret or hash).
#[derive(Serialize)]
pub struct TokenRecordDto {
    /// The token id.
    pub id: String,
    /// The human label.
    pub name: String,
    /// The granted scopes (wire names).
    pub scopes: Vec<String>,
    /// Creation time, unix milliseconds.
    pub created_at: u64,
    /// Revocation time, if revoked.
    pub revoked_at: Option<u64>,
    /// Last-used time, if ever.
    pub last_used_at: Option<u64>,
}

fn scope_name(scope: &TokenScope) -> &'static str {
    match scope {
        TokenScope::Read => "read",
        TokenScope::Send => "send",
        TokenScope::Admin => "admin",
        _ => "unknown",
    }
}

impl From<TokenRecord> for TokenRecordDto {
    fn from(r: TokenRecord) -> TokenRecordDto {
        TokenRecordDto {
            id: r.id.to_string(),
            name: r.name,
            scopes: r.scopes.iter().map(|s| scope_name(s).to_string()).collect(),
            created_at: r.created_at,
            revoked_at: r.revoked_at,
            last_used_at: r.last_used_at,
        }
    }
}

/// The `POST /tokens` body.
#[derive(Deserialize, ToSchema)]
pub struct CreateBody {
    /// A human label.
    pub name: String,
    /// The scopes to grant (`read`, `send`, `admin`).
    pub scopes: Vec<String>,
}

/// The `POST /tokens` response: the metadata plus the one-time secret.
#[derive(Serialize)]
pub struct CreatedToken {
    /// The token metadata.
    pub token: TokenRecordDto,
    /// The secret, shown once and never recoverable.
    pub secret: String,
}

/// Lists all tokens (metadata only).
#[utoipa::path(
    get,
    path = "/api/v1/tokens",
    responses((status = 200, description = "Token metadata"))
)]
pub async fn list_tokens(
    State(state): State<AppState>,
    Extension(ctx): Extension<AuthCtx>,
) -> ApiResult<Json<Vec<TokenRecordDto>>> {
    ctx.scopes.require(Scope::Read)?;
    let tokens = state
        .control
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .token_list()
        .map_err(|e| ApiError::internal(format!("token list failed: {e}")))?;
    Ok(Json(tokens.into_iter().map(TokenRecordDto::from).collect()))
}

/// Mints a token, returning its secret exactly once.
#[utoipa::path(
    post,
    path = "/api/v1/tokens",
    responses(
        (status = 200, description = "The token and its one-time secret"),
        (status = 400, description = "An unknown scope was requested")
    )
)]
pub async fn create_token(
    State(state): State<AppState>,
    Extension(ctx): Extension<AuthCtx>,
    Json(body): Json<CreateBody>,
) -> ApiResult<Json<CreatedToken>> {
    ctx.scopes.require(Scope::Admin)?;
    let mut scopes = Vec::with_capacity(body.scopes.len());
    for s in &body.scopes {
        scopes.push(match s.as_str() {
            "read" => TokenScope::Read,
            "send" => TokenScope::Send,
            "admin" => TokenScope::Admin,
            other => {
                return Err(ApiError::bad_request(
                    "bad_scope",
                    format!("unknown scope: {other}"),
                ))
            }
        });
    }
    let (record, secret) = state
        .control
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .token_create(&body.name, &scopes)
        .map_err(|e| ApiError::internal(format!("token create failed: {e}")))?;
    let _ = state
        .store
        .audit(&ctx.actor, "token.create", &record.id.to_string(), "ok");
    Ok(Json(CreatedToken {
        token: TokenRecordDto::from(record),
        secret,
    }))
}

/// Revokes a token.
#[utoipa::path(
    delete,
    path = "/api/v1/tokens/{id}",
    responses(
        (status = 200, description = "Revoked"),
        (status = 404, description = "No such active token")
    )
)]
pub async fn revoke_token(
    State(state): State<AppState>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    ctx.scopes.require(Scope::Admin)?;
    let id = parse_ulid(&id)?;
    let revoked = state
        .control
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .token_revoke(id)
        .map_err(|e| ApiError::internal(format!("token revoke failed: {e}")))?;
    let _ = state.store.audit(
        &ctx.actor,
        "token.revoke",
        &id.to_string(),
        if revoked { "ok" } else { "noop" },
    );
    if revoked {
        Ok(StatusCode::OK)
    } else {
        Err(ApiError::not_found("not_found", "no such active token"))
    }
}
