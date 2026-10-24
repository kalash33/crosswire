//! The API error type and its stable JSON body.
//!
//! Every error the API returns, including `421`, `401`, `403` and `422`,
//! serialises as `{"error":{"code":"<snake_case>","message":"..."}}`.

use axum::response::{IntoResponse, Response};
use axum::Json;
use http::StatusCode;
use serde::Serialize;

/// An API error: an HTTP status, a stable snake_case `code`, and a human
/// message. It renders as the stable error envelope.
#[derive(Clone, Debug)]
pub struct ApiError {
    /// The HTTP status code to return.
    pub status: StatusCode,
    /// The stable machine-readable code (snake_case).
    pub code: &'static str,
    /// A human-readable message. Never contains captured bytes or secrets.
    pub message: String,
}

/// The `error` field of the envelope.
#[derive(Serialize)]
struct Body {
    error: Inner,
}

#[derive(Serialize)]
struct Inner {
    code: &'static str,
    message: String,
}

impl ApiError {
    /// Builds an error from its parts.
    #[must_use]
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> ApiError {
        ApiError {
            status,
            code,
            message: message.into(),
        }
    }

    /// `400 Bad Request`.
    #[must_use]
    pub fn bad_request(code: &'static str, message: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::BAD_REQUEST, code, message)
    }

    /// `401 Unauthorized`.
    #[must_use]
    pub fn unauthorized(code: &'static str, message: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::UNAUTHORIZED, code, message)
    }

    /// `403 Forbidden`.
    #[must_use]
    pub fn forbidden(code: &'static str, message: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::FORBIDDEN, code, message)
    }

    /// `404 Not Found`.
    #[must_use]
    pub fn not_found(code: &'static str, message: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::NOT_FOUND, code, message)
    }

    /// `409 Conflict`.
    #[must_use]
    pub fn conflict(code: &'static str, message: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::CONFLICT, code, message)
    }

    /// `413 Payload Too Large`.
    #[must_use]
    pub fn too_large(code: &'static str, message: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, code, message)
    }

    /// `421 Misdirected Request`.
    #[must_use]
    pub fn misdirected(code: &'static str, message: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::MISDIRECTED_REQUEST, code, message)
    }

    /// `422 Unprocessable Entity`.
    #[must_use]
    pub fn unprocessable(code: &'static str, message: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, code, message)
    }

    /// `429 Too Many Requests`.
    #[must_use]
    pub fn rate_limited(message: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::TOO_MANY_REQUESTS, "rate_limited", message)
    }

    /// `500 Internal Server Error`.
    #[must_use]
    pub fn internal(message: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", message)
    }

    /// `502 Bad Gateway`.
    #[must_use]
    pub fn upstream(message: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::BAD_GATEWAY, "upstream_error", message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = Body {
            error: Inner {
                code: self.code,
                message: self.message,
            },
        };
        (self.status, Json(body)).into_response()
    }
}

/// Shorthand result for handlers.
pub type ApiResult<T> = Result<T, ApiError>;
