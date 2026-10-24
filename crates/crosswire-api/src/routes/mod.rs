//! The HTTP route handlers, grouped by resource.

pub mod exchanges;
pub mod intercept;
pub mod misc;
pub mod repeater;
pub mod scope;
pub mod tokens;

use ulid::Ulid;

use crate::error::ApiError;

/// Parses a path segment as a ULID, returning `400 bad_id` otherwise.
pub(crate) fn parse_ulid(s: &str) -> Result<Ulid, ApiError> {
    Ulid::from_string(s).map_err(|_| ApiError::bad_request("bad_id", "the id is not a valid ULID"))
}
