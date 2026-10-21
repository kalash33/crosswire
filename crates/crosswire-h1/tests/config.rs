//! Tests for limits and parser configuration.
use crosswire_core::Profile;
use crosswire_h1::{ParseLimits, ParserConfig};

#[test]
fn default_limits_match_spec() {
    let l = ParseLimits::default();
    assert_eq!(
        (
            l.head_size,
            l.max_headers,
            l.max_line,
            l.max_chunk_size_line,
            l.max_trailer_size
        ),
        (65536, 256, 16384, 4096, 16384)
    );
}

#[test]
fn builder_overrides() {
    let c = ParserConfig::builder()
        .profile(Profile::Strict)
        .limits(ParseLimits::builder().head_size(1024).build())
        .build();
    assert_eq!(c.profile(), Profile::Strict);
    assert_eq!(c.limits().head_size, 1024);
}
