//! Coverage of the program-list importer.

use std::collections::BTreeSet;

use crosswire_scope::{
    parse_program_list, HostPattern, ImportError, PathMatch, Rule, Scheme, Scope, ScopeError,
};

fn bare(host: HostPattern) -> Rule {
    Rule {
        host,
        ports: None,
        scheme: None,
        path: None,
    }
}

#[test]
fn every_supported_form() {
    let text = "\
# a comment, ignored

example.com
*.wild.com
https://api.example.com:8443/v1
10.0.0.0/8
1.2.3.4
[::1]
";
    let got = parse_program_list(text).expect("parse");
    let want = Scope {
        include: vec![
            bare(HostPattern::Exact("example.com".to_string())),
            bare(HostPattern::Suffix {
                suffix: "wild.com".to_string(),
                also_apex: false,
            }),
            Rule {
                host: HostPattern::Exact("api.example.com".to_string()),
                ports: Some(BTreeSet::from([8443u16])),
                scheme: Some(Scheme::Https),
                path: Some(PathMatch::Prefix("/v1".to_string())),
            },
            bare(HostPattern::Cidr("10.0.0.0/8".parse().unwrap())),
            bare(HostPattern::Ip("1.2.3.4".parse().unwrap())),
            bare(HostPattern::Ip("::1".parse().unwrap())),
        ],
        exclude: vec![],
    };
    assert_eq!(got, want);
}

#[test]
fn bad_line_reports_line_number() {
    let text = "good.com\nftp://example.com\n";
    let err = parse_program_list(text).unwrap_err();
    assert!(
        matches!(err, ImportError::Line { line: 2, .. }),
        "got {err:?}"
    );
}

#[test]
fn bad_cidr_reports_line_number() {
    let text = "good.com\n\n10.0.0.0/99\n";
    let err = parse_program_list(text).unwrap_err();
    assert!(
        matches!(err, ImportError::Line { line: 3, .. }),
        "got {err:?}"
    );
}

#[test]
fn regex_over_size_limit() {
    let scope = Scope {
        include: vec![bare(HostPattern::Regex("(?:a{1000}){1000}".to_string()))],
        exclude: vec![],
    };
    let err = scope.compile().unwrap_err();
    assert!(matches!(err, ScopeError::RegexTooLarge), "got {err:?}");
}

#[test]
fn invalid_regex_reports_error() {
    let scope = Scope {
        include: vec![bare(HostPattern::Regex("(".to_string()))],
        exclude: vec![],
    };
    let err = scope.compile().unwrap_err();
    assert!(matches!(err, ScopeError::Regex(_)), "got {err:?}");
}
