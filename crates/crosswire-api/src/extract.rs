//! Extractors whose rejections render as the stable error envelope.
//!
//! axum's built-in `Json`, `Query` and `Path` extractors reject with their own
//! plain-text bodies. These thin wrappers delegate to them but map any
//! rejection to an [`ApiError`], so a malformed body, query or path segment
//! still returns `{"error":{"code":"<snake_case>","message":"..."}}`. They keep
//! the same type names (`Json`, `Query`, `Path`) so the utoipa path macro still
//! recognises them as the request body, query parameters and path parameters.

use axum::extract::{FromRequest, FromRequestParts, Request};
use axum::response::{IntoResponse, Response};
use http::request::Parts;
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::error::ApiError;

/// A JSON body extractor whose rejection is the error envelope, and which
/// renders a serialisable value as a JSON response.
pub struct Json<T>(pub T);

impl<T, S> FromRequest<S> for Json<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match axum::Json::<T>::from_request(req, state).await {
            Ok(axum::Json(value)) => Ok(Json(value)),
            Err(rejection) => Err(ApiError::new(
                rejection.status(),
                "invalid_body",
                rejection.body_text(),
            )),
        }
    }
}

impl<T: Serialize> IntoResponse for Json<T> {
    fn into_response(self) -> Response {
        axum::Json(self.0).into_response()
    }
}

/// A query extractor whose rejection is the error envelope.
pub struct Query<T>(pub T);

impl<T, S> FromRequestParts<S> for Query<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match axum::extract::Query::<T>::from_request_parts(parts, state).await {
            Ok(axum::extract::Query(value)) => Ok(Query(value)),
            Err(rejection) => Err(ApiError::new(
                rejection.status(),
                "invalid_query",
                rejection.body_text(),
            )),
        }
    }
}

/// A path extractor whose rejection is the error envelope.
pub struct Path<T>(pub T);

impl<T, S> FromRequestParts<S> for Path<T>
where
    T: DeserializeOwned + Send,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match axum::extract::Path::<T>::from_request_parts(parts, state).await {
            Ok(axum::extract::Path(value)) => Ok(Path(value)),
            Err(rejection) => Err(ApiError::new(
                rejection.status(),
                "invalid_path",
                rejection.body_text(),
            )),
        }
    }
}
