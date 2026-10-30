//! The 0.1.0 wire freeze (design spec section 6.1, R1 schema freeze).
//!
//! `fixtures/exchange.v2.json` is one complete `Exchange` touching every
//! enum family on the wire. It must deserialise and re-serialise to exactly
//! the committed bytes: the committed file IS the serializer's canonical
//! form (`serde_json::to_value`, whose maps keep keys sorted, printed with
//! `to_string_pretty` plus a trailing newline). A test failure here means
//! the wire changed; changing the wire means editing the fixture on purpose
//! (and following the additive-only rule).
mod common;
use common::*;
use crosswire_core::{ChangeOp, ChangeReason, DiagnosticCode, EncodingHint, Severity};
use serde::de::DeserializeOwned;
use serde_json::Value;

const FIXTURE: &str = include_str!("fixtures/exchange.v2.json");

/// The canonical text of an exchange: sorted keys, pretty, newline-ended.
fn canonical(ex: &Exchange) -> String {
    let v = serde_json::to_value(ex).unwrap();
    let mut s = serde_json::to_string_pretty(&v).unwrap();
    s.push('\n');
    s
}

#[test]
fn frozen_exchange_fixture_round_trips_byte_identical() {
    let ex: Exchange = serde_json::from_str(FIXTURE).expect("fixture reads");
    let fixture_value: Value = serde_json::from_str(FIXTURE).unwrap();
    let out = serde_json::to_value(&ex).unwrap();
    assert_eq!(out, fixture_value);
    assert_eq!(
        serde_json::to_string(&out).unwrap(),
        serde_json::to_string(&fixture_value).unwrap()
    );
    let text = canonical(&ex);
    assert_eq!(
        text, FIXTURE,
        "the wire changed; canonical form of the fixture is:\n{text}"
    );

    // Every family is really exercised (an `unknown` would round-trip too).
    let req = ex.request();
    assert_eq!(req.kind(), MessageKind::Http1Raw);
    let Head::Http1Raw {
        spans,
        parser_version,
        ..
    } = req.head()
    else {
        panic!("http1-raw request head expected")
    };
    assert_eq!(*parser_version, HTTP1_PARSER_VERSION);
    assert!(matches!(spans.start_line(), StartLineSpans::Request { .. }));
    assert!(matches!(spans.framing(), FramingRecord::Ambiguous { .. }));
    let sev: Vec<Severity> = req.diagnostics().iter().map(|d| d.severity()).collect();
    assert_eq!(sev, [Severity::Info, Severity::Warning, Severity::Error]);
    let codes: Vec<DiagnosticCode> = req.diagnostics().iter().map(|d| d.code()).collect();
    assert_eq!(
        codes,
        [
            DiagnosticCode::NonCanonicalHeaderCase,
            DiagnosticCode::DuplicateContentLength,
            DiagnosticCode::Expect100Continue,
        ]
    );
    assert_eq!(req.diffs()[0].reason(), &ChangeReason::IntruderPosition(3));
    assert_eq!(req.diffs()[0].op(), ChangeOp::Insert);

    let resp = ex.response().expect("response");
    assert_eq!(resp.kind(), MessageKind::Tunnel);
    assert_eq!(resp.fidelity(), Fidelity::Opaque);
    assert!(matches!(
        resp.head(),
        Head::Tunnel {
            reason: TunnelReason::Ws,
            ..
        }
    ));
    assert_eq!(resp.framing(), &FramingRecord::Upgrade);
    assert!(matches!(
        resp.body(),
        Body::Blob {
            truncated_at: Some(_),
            capture_dropped: true,
            ..
        }
    ));
    assert_eq!(resp.diffs()[0].op(), ChangeOp::Replace);
    assert_eq!(resp.diffs()[0].reason(), &ChangeReason::InterceptEdit);
    assert!(matches!(
        resp.connection().upstream_chain_result(),
        ChainResult::Overridden { .. }
    ));
    assert!(resp.connection().cert_override());
    assert_eq!(ex.scope_state(), ScopeState::In);
    assert_eq!(ex.namespace().kind(), NamespaceKind::History);
    assert_eq!(ex.state(), ExchangeState::Committed);
    assert!(!ex.tags().is_empty() && !ex.notes().is_empty());
    assert_ne!(ex.timing(), Timing::default());
}

#[test]
fn altered_fixture_is_detected() {
    let altered = FIXTURE.replacen("\"replace\"", "\"replaced\"", 1);
    assert_ne!(altered, FIXTURE, "the fixture carries a replace op");
    let ex: Exchange = serde_json::from_str(&altered).expect("unknown codes never fail a read");
    assert_eq!(ex.response().unwrap().diffs()[0].op(), ChangeOp::Unknown);
    let altered_value: Value = serde_json::from_str(&altered).unwrap();
    assert_ne!(serde_json::to_value(&ex).unwrap(), altered_value);
    assert_ne!(canonical(&ex), altered);
}

fn read<T: DeserializeOwned>(json: &str) -> T {
    serde_json::from_str(json).unwrap_or_else(|e| panic!("{json}: {e}"))
}

#[test]
fn every_wire_enum_reads_unknown_codes() {
    // crosswire-core, plain string codes.
    assert_eq!(read::<ChangeOp>(r#""no-such-code""#), ChangeOp::Unknown);
    assert_eq!(read::<ChangeOp>(r#""insert""#), ChangeOp::Insert);
    assert_eq!(read::<ChangeOp>(r#""delete""#), ChangeOp::Delete);
    assert_eq!(read::<ChangeOp>(r#""replace""#), ChangeOp::Replace);

    assert_eq!(
        read::<ChangeReason>(r#""no-such-code""#),
        ChangeReason::Unknown
    );
    assert_eq!(
        read::<ChangeReason>(r#""hop-by-hop""#),
        ChangeReason::HopByHop
    );
    assert_eq!(
        read::<ChangeReason>(r#""content-length""#),
        ChangeReason::ContentLength
    );
    assert_eq!(
        read::<ChangeReason>(r#""intercept-edit""#),
        ChangeReason::InterceptEdit
    );
    assert_eq!(
        read::<ChangeReason>(r#"{"intruder-position":3}"#),
        ChangeReason::IntruderPosition(3)
    );
    assert_eq!(
        read::<ChangeReason>(r#"{"match-replace":{"rule":"r"}}"#),
        ChangeReason::MatchReplace { rule: "r".into() }
    );
    assert_eq!(
        read::<ChangeReason>(r#"{"custom":"why"}"#),
        ChangeReason::Custom("why".into())
    );
    assert_eq!(
        serde_json::to_string(&ChangeReason::IntruderPosition(3)).unwrap(),
        r#"{"intruder-position":3}"#
    );

    assert_eq!(
        read::<EncodingHint>(r#""no-such-code""#),
        EncodingHint::Unknown
    );
    assert_eq!(read::<EncodingHint>(r#""indexed""#), EncodingHint::Indexed);
    assert_eq!(
        read::<EncodingHint>(r#""literal-with-indexing""#),
        EncodingHint::LiteralWithIndexing
    );
    assert_eq!(
        read::<EncodingHint>(r#""literal-without-indexing""#),
        EncodingHint::LiteralWithoutIndexing
    );
    assert_eq!(
        read::<EncodingHint>(r#""never-indexed""#),
        EncodingHint::NeverIndexed
    );

    assert_eq!(read::<Severity>(r#""no-such-code""#), Severity::Unknown);
    assert_eq!(read::<Severity>(r#""info""#), Severity::Info);
    assert_eq!(read::<Severity>(r#""warning""#), Severity::Warning);
    assert_eq!(read::<Severity>(r#""error""#), Severity::Error);

    assert_eq!(
        read::<DiagnosticCode>(r#""no-such-code""#),
        DiagnosticCode::Unknown
    );
    assert_eq!(
        read::<DiagnosticCode>(r#""duplicate_content_length""#),
        DiagnosticCode::DuplicateContentLength
    );
    assert_eq!(
        read::<DiagnosticCode>(r#""http_0_9_request""#),
        DiagnosticCode::Http09Request
    );

    // crosswire-model, `type`-tagged.
    assert_eq!(
        read::<StartLineSpans>(r#"{"type":"no-such-type"}"#),
        StartLineSpans::Unknown
    );
    assert!(matches!(
        read::<StartLineSpans>(
            r#"{"type":"response","version":{"start":0,"end":8},"status":{"start":9,"end":12},"reason":{"start":13,"end":15}}"#
        ),
        StartLineSpans::Response { .. }
    ));

    assert_eq!(
        read::<FramingRecord>(r#"{"type":"no-such-type","len":1}"#),
        FramingRecord::Unknown
    );
    assert_eq!(
        read::<FramingRecord>(r#"{"type":"content-length","len":4}"#),
        FramingRecord::ContentLength { len: 4 }
    );
    assert_eq!(
        read::<FramingRecord>(r#"{"type":"until-close"}"#),
        FramingRecord::UntilClose
    );

    assert_eq!(read::<Head>(r#"{"type":"h9-frames","x":1}"#), Head::Unknown);
    assert!(matches!(
        read::<Head>(r#"{"type":"tunnel","reason":"ws"}"#),
        Head::Tunnel {
            bytes_in: 0,
            bytes_out: 0,
            duration_ms: 0,
            reason: TunnelReason::Ws
        }
    ));

    assert_eq!(read::<Body>(r#"{"type":"no-such-type"}"#), Body::Unknown);
    assert_eq!(read::<Body>(r#"{"type":"none"}"#), Body::None);

    assert_eq!(
        read::<ChainResult>(r#"{"type":"no-such-type","reason":"r"}"#),
        ChainResult::Unknown
    );
    assert_eq!(
        read::<ChainResult>(r#"{"type":"valid"}"#),
        ChainResult::Valid
    );
    assert_eq!(
        read::<ChainResult>(r#"{"type":"not-tls"}"#),
        ChainResult::NotTls
    );

    let ev = read::<Event>(r#"{"type":"no-such-type","seq":5}"#);
    assert_eq!(ev, Event::Unknown);
    assert_eq!(ev.seq(), 0);
    assert!(matches!(
        read::<Event>(r#"{"type":"lagged","missed":2,"seq":5}"#),
        Event::Lagged { missed: 2, seq: 5 }
    ));

    // crosswire-model, plain string codes.
    assert_eq!(
        read::<ExchangeState>(r#""no-such-code""#),
        ExchangeState::Unknown
    );
    assert_eq!(read::<ExchangeState>(r#""error""#), ExchangeState::Error);
    assert_eq!(read::<ScopeState>(r#""no-such-code""#), ScopeState::Unknown);
    assert_eq!(
        read::<ScopeState>(r#""host-mismatch""#),
        ScopeState::HostMismatch
    );
    assert_eq!(
        read::<NamespaceKind>(r#""no-such-code""#),
        NamespaceKind::Unknown
    );
    assert_eq!(
        read::<NamespaceKind>(r#""intruder""#),
        NamespaceKind::Intruder
    );
}

#[test]
fn diagnostic_wire_codes_equal_as_str() {
    for c in DiagnosticCode::ALL {
        assert_eq!(
            serde_json::to_string(c).unwrap(),
            format!("\"{}\"", c.as_str()),
            "{c:?}"
        );
        assert_eq!(read::<DiagnosticCode>(&format!("\"{}\"", c.as_str())), *c);
    }
    assert!(!DiagnosticCode::ALL.contains(&DiagnosticCode::Unknown));
    for s in [
        Severity::Info,
        Severity::Warning,
        Severity::Error,
        Severity::Unknown,
    ] {
        assert_eq!(
            serde_json::to_string(&s).unwrap(),
            format!("\"{}\"", s.as_str()),
            "{s:?}"
        );
    }
}

#[test]
fn cert_override_is_derived_from_chain_result() {
    let with = |c: ChainResult| ConnectionInfo::new("x", 443).with_chain_result(c);
    assert!(with(ChainResult::Overridden {
        reason: "UnknownIssuer".into()
    })
    .cert_override());
    assert!(!with(ChainResult::Valid).cert_override());
    assert!(!with(ChainResult::Invalid {
        reason: "bad".into()
    })
    .cert_override());
    assert!(!with(ChainResult::NotTls).cert_override());

    let v = serde_json::to_value(with(ChainResult::Overridden {
        reason: String::new(),
    }))
    .unwrap();
    assert!(v.get("cert_override").is_none(), "{v}");

    // Pre-freeze data carrying the old key still reads; the key is ignored.
    let old: ConnectionInfo = read(
        r#"{"target_addr":"x","port":443,"upstream_chain_result":{"type":"valid"},"cert_override":true}"#,
    );
    assert!(!old.cert_override());
}
