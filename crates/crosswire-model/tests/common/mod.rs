//! Shared helpers for the integration tests.
#![allow(dead_code)]

pub use crosswire_h1::{Parser, ParserConfig, Progress, RawMessage, ResponseContext};
pub use crosswire_model::*;

/// Parses one complete request head (plus nothing else) into a `RawMessage`.
pub fn parse_req(input: &[u8]) -> RawMessage {
    let mut p = Parser::request(&ParserConfig::default());
    match p.feed(input).unwrap().event {
        Progress::Head(m) => m,
        other => panic!("expected head, got {other:?}"),
    }
}

/// Parses one complete response head into a `RawMessage`.
pub fn parse_resp(input: &[u8]) -> RawMessage {
    let mut p = Parser::response(
        &ParserConfig::default(),
        ResponseContext::for_method(b"GET"),
    );
    match p.feed(input).unwrap().event {
        Progress::Head(m) => m,
        other => panic!("expected head, got {other:?}"),
    }
}

/// A plain connection record.
pub fn conn(host: &str, port: u16) -> ConnectionInfo {
    ConnectionInfo::new(host, port)
}

/// A small request-only exchange.
pub fn sample_exchange() -> Exchange {
    let raw = parse_req(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n");
    Exchange::with_id(
        ulid::Ulid::from_parts(1_700_000_000_000, 42),
        Message::http1_raw(&raw, Body::None, conn("x", 80)),
    )
}
