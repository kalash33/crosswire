//! Tests for the diagnostic catalogue.
use crosswire_core::{DiagnosticCode, Profile, Severity};
#[test]
fn catalogue_has_26_stable_names() {
    assert_eq!(DiagnosticCode::ALL.len(), 26);
    let names: Vec<&str> = DiagnosticCode::ALL.iter().map(|c| c.as_str()).collect();
    assert_eq!(names[0], "duplicate_content_length");
    assert_eq!(names[24], "version_unusual");
    assert_eq!(names[25], "chunk_size_whitespace");
    let mut sorted = names.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), 26);
}
#[test]
fn severities_follow_spec_table() {
    assert_eq!(
        DiagnosticCode::BareLf.default_severity(Profile::Lenient),
        Severity::Warning
    );
    assert_eq!(
        DiagnosticCode::BareLf.default_severity(Profile::Strict),
        Severity::Error
    );
    assert_eq!(
        DiagnosticCode::NonCanonicalHeaderCase.default_severity(Profile::Strict),
        Severity::Info
    );
    assert_eq!(
        DiagnosticCode::InvalidChunkSize.default_severity(Profile::Lenient),
        Severity::Error
    );
}
