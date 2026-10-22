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
