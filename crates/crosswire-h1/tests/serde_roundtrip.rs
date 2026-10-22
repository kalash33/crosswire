//! Serde round-trips for the plain-data types.
#![cfg(feature = "serde")]

use crosswire_h1::*;

fn rt<T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + core::fmt::Debug>(v: T) {
    let s = serde_json::to_string(&v).unwrap();
    assert_eq!(serde_json::from_str::<T>(&s).unwrap(), v);
}

#[test]
fn framing_roundtrip() {
    rt(BodyFraming::Ambiguous {
        resolved: Box::new(BodyFraming::Chunked),
        candidates: vec![BodyFraming::Chunked, BodyFraming::ContentLength(5)],
        rule: ResolutionRule::TransferEncodingWins,
    });
    rt(BodyFraming::None);
}

#[test]
fn limits_and_context_roundtrip() {
    rt(ParseLimits::default());
    rt(ResponseContext::for_method(b"HEAD"));
    rt(Kind::Response);
    rt(Method::Other);
}

#[test]
fn spans_roundtrip() {
    let mut p = Parser::request(&ParserConfig::default());
    let Progress::Head(m) = p.feed(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n").unwrap().event else {
        panic!()
    };
    rt(*m.head().start());
    rt(m.head().headers()[0]);
}
