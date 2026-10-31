//! Unknown-field tolerance tests.
mod common;
use common::*;

#[test]
fn unknown_fields_are_ignored() {
    let mut v: serde_json::Value = serde_json::to_value(sample_exchange()).unwrap();
    v["future_field"] = serde_json::json!(42);
    v["request"]["future_nested"] = serde_json::json!({"a": 1});
    let back: Exchange = serde_json::from_value(v).unwrap();
    assert_eq!(back.id(), sample_exchange().id());
}

#[test]
fn unknown_fields_ignored_in_nested_enums_and_events() {
    let mut v = serde_json::to_value(sample_exchange()).unwrap();
    v["request"]["head"]["future"] = serde_json::json!(true);
    v["request"]["body"] = serde_json::json!({"type": "none", "future": 1});
    v["request"]["connection"]["future"] = serde_json::json!("x");
    let back: Exchange = serde_json::from_value(v).unwrap();
    assert_eq!(back.request().body(), &Body::None);

    let ev: Event =
        serde_json::from_str(r#"{"type":"error","detail":"d","seq":4,"future":[1]}"#).unwrap();
    assert_eq!(ev.seq(), 4);
}

#[test]
fn missing_optional_fields_default() {
    let mut v = serde_json::to_value(sample_exchange()).unwrap();
    let o = v.as_object_mut().unwrap();
    for k in [
        "response",
        "timing",
        "tags",
        "notes",
        "namespace",
        "scope_state",
        "state",
    ] {
        o.remove(k);
    }
    let back: Exchange = serde_json::from_value(v).unwrap();
    assert!(back.response().is_none());
    assert_eq!(back.state(), ExchangeState::Received);
    assert_eq!(back.scope_state(), ScopeState::Empty);
    assert_eq!(back.namespace().kind(), NamespaceKind::History);
}

#[test]
fn unknown_simple_enum_variants_read_as_unknown() {
    assert_eq!(
        serde_json::from_str::<Fidelity>("\"quantum\"").unwrap(),
        Fidelity::Unknown
    );
    assert_eq!(
        serde_json::from_str::<MessageKind>("\"h3\"").unwrap(),
        MessageKind::Unknown
    );
    assert_eq!(
        serde_json::from_str::<MarkerKind>("\"x\"").unwrap(),
        MarkerKind::Unknown
    );
    assert_eq!(
        serde_json::from_str::<TunnelReason>("\"x\"").unwrap(),
        TunnelReason::Unknown
    );
    assert_eq!(
        serde_json::from_str::<Fidelity>("\"raw\"").unwrap(),
        Fidelity::Raw
    );
}

#[test]
fn missing_parser_version_and_seq_default() {
    let mut v = serde_json::to_value(sample_exchange()).unwrap();
    v["request"]["head"]
        .as_object_mut()
        .unwrap()
        .remove("parser_version")
        .expect("parser_version is written");
    let back: Exchange = serde_json::from_value(v).unwrap();
    let Head::Http1Raw { parser_version, .. } = back.request().head() else {
        panic!("http1-raw expected")
    };
    assert_eq!(*parser_version, HTTP1_PARSER_VERSION);

    let id = r#""01HF0000000000000000000000""#;
    for json in [
        format!(r#"{{"type":"received","id":{id}}}"#),
        r#"{"type":"committed"}"#.to_string(),
        r#"{"type":"error"}"#.to_string(),
        r#"{"type":"limit-exceeded"}"#.to_string(),
        r#"{"type":"lagged","missed":1}"#.to_string(),
    ] {
        let ev: Event = serde_json::from_str(&json).unwrap_or_else(|e| panic!("{json}: {e}"));
        assert_eq!(ev.seq(), 0, "{json}");
    }

    let h: Head = serde_json::from_str(r#"{"type":"tunnel","reason":"ws"}"#).unwrap();
    assert!(matches!(
        h,
        Head::Tunnel {
            bytes_in: 0,
            bytes_out: 0,
            duration_ms: 0,
            ..
        }
    ));
}
