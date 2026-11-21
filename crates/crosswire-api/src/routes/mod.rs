//! The HTTP route handlers, grouped by resource.

pub mod exchanges;
pub mod intercept;
pub mod misc;
pub mod preview;
pub mod repeater;
pub mod scope;
pub mod tokens;

use ulid::Ulid;

use crate::error::ApiError;

/// Parses a path segment as a ULID, returning `400 bad_id` otherwise.
///
/// Only canonical text is accepted (either case): the ulid crate drops the
/// bits past 128 when the first character is above `7`, so without the
/// round-trip check every id would have aliases.
pub(crate) fn parse_ulid(s: &str) -> Result<Ulid, ApiError> {
    Ulid::from_string(s)
        .ok()
        .filter(|id| id.to_string().eq_ignore_ascii_case(s))
        .ok_or_else(|| ApiError::bad_request("bad_id", "the id is not a valid ULID"))
}

#[cfg(test)]
mod tests {
    use super::parse_ulid;
    use ulid::Ulid;

    #[test]
    fn parse_ulid_accepts_canonical_text_in_either_case() {
        assert_eq!(
            parse_ulid("00000000000000000000000000").ok(),
            Some(Ulid::nil())
        );
        assert_eq!(
            parse_ulid("01aryz6s410000000000000000").ok(),
            Some(Ulid::from_parts(1_469_918_176_385, 0))
        );
    }

    #[test]
    fn parse_ulid_rejects_overflow_aliases() {
        // The ulid crate drops the bits past 128, so this parses as the
        // nil id; it is not that id's text and must be refused.
        let err = parse_ulid("80000000000000000000000000").err();
        assert_eq!(err.map(|e| e.code), Some("bad_id"));
    }
}
