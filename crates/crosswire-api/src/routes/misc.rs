//! Unauthenticated routes: health, the OpenAPI document, the CA certificate
//! downloads, and the bootstrap session exchange.

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, State};
use axum::response::IntoResponse;
use axum::Json;
use http::header::{CONTENT_TYPE, SET_COOKIE};
use http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::app::AppState;
use crate::error::{ApiError, ApiResult};

/// The health response body.
#[derive(Serialize, ToSchema)]
pub struct HealthResponse {
    /// Always `"ok"` while the API is serving.
    pub status: &'static str,
}

/// Liveness probe.
#[utoipa::path(
    get,
    path = "/api/v1/health",
    responses((status = 200, description = "The API is serving", body = HealthResponse))
)]
pub async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

/// The OpenAPI document for the API.
#[utoipa::path(
    get,
    path = "/api/v1/openapi.json",
    responses((status = 200, description = "The OpenAPI document"))
)]
pub async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(crate::openapi::document())
}

/// The CA certificate in DER form.
#[utoipa::path(
    get,
    path = "/api/v1/ca.der",
    responses((status = 200, description = "The CA certificate, DER", content_type = "application/pkix-cert"))
)]
pub async fn ca_der(State(state): State<AppState>) -> impl IntoResponse {
    (
        StatusCode::OK,
        [(CONTENT_TYPE, "application/pkix-cert")],
        state.ca.cert_der(),
    )
}

/// The CA certificate in PEM form.
#[utoipa::path(
    get,
    path = "/api/v1/ca.pem",
    responses((status = 200, description = "The CA certificate, PEM", content_type = "application/x-pem-file"))
)]
pub async fn ca_pem(State(state): State<AppState>) -> impl IntoResponse {
    (
        StatusCode::OK,
        [(CONTENT_TYPE, "application/x-pem-file")],
        state.ca.cert_pem(),
    )
}

/// The bootstrap request body.
#[derive(Deserialize, ToSchema)]
pub struct SessionRequest {
    /// The single-use bootstrap token.
    pub bootstrap: String,
}

/// The bootstrap response body.
#[derive(Serialize, ToSchema)]
pub struct SessionResponse {
    /// Seconds until the session expires if idle.
    pub expires_in: u64,
}

/// Exchanges the single-use bootstrap token for an operator session cookie.
#[utoipa::path(
    post,
    path = "/api/v1/session",
    request_body = SessionRequest,
    responses(
        (status = 200, description = "A session was created", body = SessionResponse),
        (status = 401, description = "The bootstrap token was wrong, expired or reused"),
        (status = 429, description = "Too many bootstrap failures")
    )
)]
pub async fn create_session(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Json(body): Json<SessionRequest>,
) -> ApiResult<impl IntoResponse> {
    let ip = peer.ip();
    if state.auth_limiter.over_limit(ip) {
        let _ = state
            .store
            .audit(&ip.to_string(), "session.bootstrap", "", "fail");
        return Err(ApiError::rate_limited("too many bootstrap attempts"));
    }
    if state.bootstrap.consume(&body.bootstrap) {
        let id = state.sessions.create();
        let _ = state
            .store
            .audit(&ip.to_string(), "session.bootstrap", "", "ok");
        let cookie = format!(
            "cw_sid_{}={id}; HttpOnly; SameSite=Strict; Path=/api",
            state.port
        );
        let body = SessionResponse {
            expires_in: state.idle_timeout.as_secs(),
        };
        Ok((StatusCode::OK, [(SET_COOKIE, cookie)], Json(body)))
    } else {
        state.auth_limiter.record(ip);
        let _ = state
            .store
            .audit(&ip.to_string(), "session.bootstrap", "", "fail");
        Err(ApiError::unauthorized(
            "bootstrap_invalid",
            "the bootstrap token is wrong, expired or already used",
        ))
    }
}
