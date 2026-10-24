//! Pins the persisted JSON shape of the scope types (Tasks 4-6 store a
//! `Scope` in the project) and checks value round-trips.

use std::collections::BTreeSet;

use crosswire_scope::{HostPattern, PathMatch, Rule, Scheme, Scope};
use serde_json::{json, Value};

fn round_trip_host(value: &HostPattern) {
    let json = serde_json::to_value(value).expect("serialize");
    let back: HostPattern = serde_json::from_value(json).expect("deserialize");
    assert_eq!(&back, value);
}

fn round_trip_path(value: &PathMatch) {
    let json = serde_json::to_value(value).expect("serialize");
    let back: PathMatch = serde_json::from_value(json).expect("deserialize");
    assert_eq!(&back, value);
}

#[test]
fn host_pattern_exact_shape() {
    let v = HostPattern::Exact("example.com".to_string());
    assert_eq!(
        serde_json::to_value(&v).unwrap(),
        json!({ "type": "exact", "value": "example.com" })
    );
    round_trip_host(&v);
}

#[test]
fn host_pattern_suffix_shapes() {
    let with_apex = HostPattern::Suffix {
        suffix: "example.com".to_string(),
        also_apex: true,
    };
    assert_eq!(
        serde_json::to_value(&with_apex).unwrap(),
        json!({ "type": "suffix", "value": { "suffix": "example.com", "also_apex": true } })
    );
    round_trip_host(&with_apex);

    let without_apex = HostPattern::Suffix {
        suffix: "example.com".to_string(),
        also_apex: false,
    };
    assert_eq!(
        serde_json::to_value(&without_apex).unwrap(),
        json!({ "type": "suffix", "value": { "suffix": "example.com", "also_apex": false } })
    );
    round_trip_host(&without_apex);
}

#[test]
fn host_pattern_regex_shape() {
    let v = HostPattern::Regex(r"^(api|www)\.example\.com$".to_string());
    assert_eq!(
        serde_json::to_value(&v).unwrap(),
        json!({ "type": "regex", "value": r"^(api|www)\.example\.com$" })
    );
    round_trip_host(&v);
}

#[test]
fn host_pattern_ip_shape() {
    let v = HostPattern::Ip("1.2.3.4".parse().unwrap());
    assert_eq!(
        serde_json::to_value(&v).unwrap(),
        json!({ "type": "ip", "value": "1.2.3.4" })
    );
    round_trip_host(&v);

    let v6 = HostPattern::Ip("::1".parse().unwrap());
    assert_eq!(
        serde_json::to_value(&v6).unwrap(),
        json!({ "type": "ip", "value": "::1" })
    );
    round_trip_host(&v6);
}

#[test]
fn host_pattern_cidr_shape() {
    let v = HostPattern::Cidr("10.0.0.0/8".parse().unwrap());
    assert_eq!(
        serde_json::to_value(&v).unwrap(),
        json!({ "type": "cidr", "value": "10.0.0.0/8" })
    );
    round_trip_host(&v);
}

#[test]
fn path_match_shapes() {
    let prefix = PathMatch::Prefix("/api".to_string());
    assert_eq!(
        serde_json::to_value(&prefix).unwrap(),
        json!({ "type": "prefix", "value": "/api" })
    );
    round_trip_path(&prefix);

    let regex = PathMatch::Regex("^/api/v[0-9]+".to_string());
    assert_eq!(
        serde_json::to_value(&regex).unwrap(),
        json!({ "type": "regex", "value": "^/api/v[0-9]+" })
    );
    round_trip_path(&regex);
}

#[test]
fn rule_omits_none_fields() {
    let rule = Rule {
        host: HostPattern::Exact("example.com".to_string()),
        ports: None,
        scheme: None,
        path: None,
    };
    let json = serde_json::to_value(&rule).unwrap();
    let obj = json.as_object().expect("object");
    assert!(obj.contains_key("host"));
    assert!(
        !obj.contains_key("ports"),
        "ports must be omitted when None"
    );
    assert!(
        !obj.contains_key("scheme"),
        "scheme must be omitted when None"
    );
    assert!(!obj.contains_key("path"), "path must be omitted when None");
    assert_eq!(
        json,
        json!({ "host": { "type": "exact", "value": "example.com" } })
    );
    let back: Rule = serde_json::from_value(json).unwrap();
    assert_eq!(back, rule);
}

#[test]
fn rule_full_shape() {
    let rule = Rule {
        host: HostPattern::Exact("api.example.com".to_string()),
        ports: Some(BTreeSet::from([80u16, 443u16])),
        scheme: Some(Scheme::Https),
        path: Some(PathMatch::Prefix("/v1".to_string())),
    };
    let json = serde_json::to_value(&rule).unwrap();
    assert_eq!(
        json,
        json!({
            "host": { "type": "exact", "value": "api.example.com" },
            "ports": [80, 443],
            "scheme": "https",
            "path": { "type": "prefix", "value": "/v1" }
        })
    );
    let back: Rule = serde_json::from_value(json).unwrap();
    assert_eq!(back, rule);
}

#[test]
fn scope_round_trip_with_include_and_exclude() {
    let scope = Scope {
        include: vec![
            Rule {
                host: HostPattern::Suffix {
                    suffix: "example.com".to_string(),
                    also_apex: true,
                },
                ports: None,
                scheme: None,
                path: None,
            },
            Rule {
                host: HostPattern::Cidr("10.0.0.0/8".parse().unwrap()),
                ports: Some(BTreeSet::from([8080u16])),
                scheme: Some(Scheme::Http),
                path: None,
            },
        ],
        exclude: vec![Rule {
            host: HostPattern::Exact("admin.example.com".to_string()),
            ports: None,
            scheme: None,
            path: Some(PathMatch::Regex("^/secret".to_string())),
        }],
    };
    let json = serde_json::to_value(&scope).unwrap();
    let back: Scope = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(back, scope);

    // The top-level shape is a plain { include: [...], exclude: [...] }.
    let obj = json.as_object().unwrap();
    assert!(matches!(obj.get("include"), Some(Value::Array(_))));
    assert!(matches!(obj.get("exclude"), Some(Value::Array(_))));
}
