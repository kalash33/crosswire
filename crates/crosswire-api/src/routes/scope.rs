//! Scope routes: read the source document, replace it, import a pasted list,
//! and toggle strict scope enforcement.

use std::sync::atomic::Ordering;
use std::sync::PoisonError;

use crate::extract::Json;
use axum::extract::State;
use axum::Extension;
use crosswire_scope::{parse_program_list, Scope};
use http::StatusCode;
use serde::Deserialize;
use utoipa::ToSchema;

use crate::app::AppState;
use crate::auth::{AuthCtx, Scope as Cap};
use crate::error::{ApiError, ApiResult};

/// The `POST /scope/import` body.
#[derive(Deserialize, ToSchema)]
pub struct ImportBody {
    /// A pasted scope list (bare hosts, wildcards, URLs, CIDR blocks).
    pub text: String,
}

/// The `PUT /settings/enforce_scope_strictly` body.
#[derive(Deserialize, ToSchema)]
pub struct EnabledBody {
    /// Whether the setting is on.
    pub enabled: bool,
}

/// Returns the current scope source document.
#[utoipa::path(
    get,
    path = "/api/v1/scope",
    responses((status = 200, description = "The scope document"))
)]
pub async fn get_scope(
    State(state): State<AppState>,
    Extension(ctx): Extension<AuthCtx>,
) -> ApiResult<Json<Scope>> {
    ctx.scopes.require(Cap::Read)?;
    let scope = state
        .scope_doc
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    Ok(Json(scope))
}

/// Replaces the scope. The new scope is compiled before anything is stored;
/// a compile error leaves the live scope untouched.
#[utoipa::path(
    put,
    path = "/api/v1/scope",
    responses(
        (status = 200, description = "The scope was replaced"),
        (status = 422, description = "The scope did not compile")
    )
)]
pub async fn put_scope(
    State(state): State<AppState>,
    Extension(ctx): Extension<AuthCtx>,
    Json(value): Json<serde_json::Value>,
) -> ApiResult<StatusCode> {
    ctx.scopes.require(Cap::Admin)?;
    let scope: Scope = serde_json::from_value(value)
        .map_err(|e| ApiError::unprocessable("scope_invalid", format!("invalid scope: {e}")))?;
    let compiled = scope.compile().map_err(|e| {
        ApiError::unprocessable("scope_invalid", format!("scope did not compile: {e}"))
    })?;
    // Serialize before the swap (and before taking any lock), so a serialize
    // failure cannot leave the live scope half-updated.
    let serialized = serde_json::to_string(&scope)
        .map_err(|e| ApiError::internal(format!("serializing the scope: {e}")))?;
    // Swap both the compiled matcher (what the proxy/repeater/intercept read)
    // and the source document (what GET returns), under their write locks.
    *state.scope.write().unwrap_or_else(PoisonError::into_inner) = compiled;
    *state
        .scope_doc
        .write()
        .unwrap_or_else(PoisonError::into_inner) = scope;
    let _ = state.store.audit(&ctx.actor, "scope.set", "", "ok");
    // Persist the source document so a later restart loads it (the server's
    // `load_scope` reads this same "scope" setting at open).
    let persisted = state
        .control
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .setting_put("scope", &serialized);
    if let Err(e) = persisted {
        tracing::warn!(key = "scope", error = %e, "scope applied but not persisted; it will revert on restart");
    }
    Ok(StatusCode::OK)
}

/// Parses a pasted scope list into a scope document, without saving it.
#[utoipa::path(
    post,
    path = "/api/v1/scope/import",
    responses(
        (status = 200, description = "The parsed scope"),
        (status = 422, description = "The list could not be parsed")
    )
)]
pub async fn import_scope(
    State(_state): State<AppState>,
    Extension(ctx): Extension<AuthCtx>,
    Json(body): Json<ImportBody>,
) -> ApiResult<Json<Scope>> {
    ctx.scopes.require(Cap::Admin)?;
    let scope = parse_program_list(&body.text)
        .map_err(|e| ApiError::unprocessable("scope_invalid", format!("import failed: {e}")))?;
    Ok(Json(scope))
}

/// Turns strict scope enforcement on or off.
#[utoipa::path(
    put,
    path = "/api/v1/settings/enforce_scope_strictly",
    responses((status = 200, description = "The setting was changed"))
)]
pub async fn set_enforce_strict(
    State(state): State<AppState>,
    Extension(ctx): Extension<AuthCtx>,
    Json(body): Json<EnabledBody>,
) -> ApiResult<StatusCode> {
    ctx.scopes.require(Cap::Admin)?;
    state.strict.store(body.enabled, Ordering::SeqCst);
    let _ = state.store.audit(
        &ctx.actor,
        "enforce_scope_strictly",
        "",
        if body.enabled { "enabled" } else { "disabled" },
    );
    // Persist the setting so a later restart keeps it.
    let persisted = state
        .control
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .setting_put("enforce_scope_strictly", &body.enabled.to_string());
    if let Err(e) = persisted {
        tracing::warn!(key = "enforce_scope_strictly", error = %e, "setting applied but not persisted; it will revert on restart");
    }
    Ok(StatusCode::OK)
}
