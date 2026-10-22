//! Keeps `docs/h1-diagnostics.md` in step with `DiagnosticCode`.

use crosswire_core::{DiagnosticCode, Profile, Severity};

fn rows() -> Vec<Vec<String>> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/h1-diagnostics.md");
    let text = std::fs::read_to_string(path).expect("docs/h1-diagnostics.md");
    text.lines()
        .filter(|l| l.starts_with('|'))
        .map(|l| {
            l.trim()
                .trim_matches('|')
                .split('|')
                .map(|c| c.trim().trim_matches('`').to_string())
                .collect::<Vec<_>>()
        })
        .filter(|cells| {
            cells[0] != "code" && !cells[0].chars().all(|c| c == '-' || c == ' ' || c == ':')
        })
        .collect()
}

fn parse_severity(s: &str) -> Severity {
    match s.to_ascii_lowercase().as_str() {
        "info" => Severity::Info,
        "warning" => Severity::Warning,
        "error" => Severity::Error,
        other => panic!("unknown severity {other:?}"),
    }
}

#[test]
fn docs_catalogue_matches_code() {
    let rows = rows();
    assert_eq!(rows.len(), 25, "table must have 25 data rows");
    assert_eq!(DiagnosticCode::ALL.len(), 25);
    for code in DiagnosticCode::ALL {
        let name = code.as_str();
        let matching: Vec<_> = rows.iter().filter(|r| r[0] == name).collect();
        assert_eq!(matching.len(), 1, "{name} must appear exactly once");
        let row = matching[0];
        assert_eq!(row.len(), 4, "{name}: expected 4 columns");
        assert_eq!(
            parse_severity(&row[2]),
            code.default_severity(Profile::Lenient),
            "{name}: Lenient severity"
        );
        assert_eq!(
            parse_severity(&row[3]),
            code.default_severity(Profile::Strict),
            "{name}: Strict severity"
        );
        assert!(!row[1].is_empty(), "{name}: meaning is empty");
    }
}
