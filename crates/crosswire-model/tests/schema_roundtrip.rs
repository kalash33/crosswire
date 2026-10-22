//! Schema round-trip tests.
mod common;
use common::*;

#[test]
fn exchange_roundtrips_json() {
    let raw = parse_req(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n");
    let ex = Exchange::new(Message::http1_raw(&raw, Body::None, conn("x", 80)));
    let s = serde_json::to_string(&ex).unwrap();
    let back: Exchange = serde_json::from_str(&s).unwrap();
    assert_eq!(back, ex);
    assert!(s.contains("\"fidelity\":\"raw\"") && s.contains("\"kind\":\"http1-raw\""));
}

#[test]
fn full_exchange_roundtrips_with_response_and_blob() {
    let req = parse_req(b"POST /a HTTP/1.1\r\nHost: x\r\nContent-Length: 2\r\n\r\n");
    let resp = parse_resp(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n");
    let mut ex = sample_exchange();
    ex = Exchange::with_id(
        ex.id(),
        Message::http1_raw(
            &req,
            Body::Inline {
                bytes: bytes::Bytes::from_static(b"\x00\xffhi"),
            },
            conn("x", 443),
        ),
    );
    let blob = Body::Blob {
        hash: [7u8; 32],
        len: 99,
        truncated_at: Some(50),
        capture_dropped: true,
    };
    ex.set_response(Message::http1_raw(&resp, blob, conn("x", 443)));
    ex.add_tag("t1");
    ex.set_notes("n");
    ex.set_state(ExchangeState::Committed);
    ex.set_scope_state(ScopeState::HostMismatch);
    let s = serde_json::to_string(&ex).unwrap();
    assert!(s.contains("\"host-mismatch\""));
    assert!(s.contains("AP9oaQ=="), "inline body is base64: {s}");
    let back: Exchange = serde_json::from_str(&s).unwrap();
    assert_eq!(back, ex);
}

#[test]
fn event_roundtrips_and_has_seq() {
    let ev = Event::Committed {
        ids: vec![sample_exchange().id()],
        seq: 9,
    };
    assert_eq!(ev.seq(), 9);
    let s = serde_json::to_string(&ev).unwrap();
    assert!(s.contains("\"type\":\"committed\""));
    assert_eq!(serde_json::from_str::<Event>(&s).unwrap(), ev);
    let m = Event::Marker {
        marker: Marker::with_id(sample_exchange().id(), MarkerKind::LimitExceeded, "too big"),
        seq: 3,
    };
    assert_eq!(m.seq(), 3);
    let s = serde_json::to_string(&m).unwrap();
    assert_eq!(serde_json::from_str::<Event>(&s).unwrap(), m);
}

#[test]
fn schema_version_is_one() {
    assert_eq!(SCHEMA_VERSION, 1);
}

#[test]
fn http1_raw_copies_head_and_diagnostics() {
    // Bare LF and a missing Host both raise diagnostics.
    let raw = parse_req(b"GET / HTTP/1.1\nX: y\n\n");
    let m = Message::http1_raw(&raw, Body::None, conn("x", 80));
    assert_eq!(m.kind(), MessageKind::Http1Raw);
    assert_eq!(m.fidelity(), Fidelity::Raw);
    assert_eq!(m.diagnostics(), raw.diagnostics());
    assert!(!m.diagnostics().is_empty());
    let Head::Http1Raw { bytes, .. } = m.head() else {
        panic!()
    };
    assert_eq!(bytes.as_ref(), raw.bytes().as_ref());
}

#[test]
fn tunnel_head_and_diff_roundtrip() {
    let h = Head::Tunnel {
        bytes_in: 1,
        bytes_out: 2,
        duration_ms: 3,
        reason: TunnelReason::SniPassthrough,
    };
    let s = serde_json::to_string(&h).unwrap();
    assert!(s.contains("sni-passthrough"));
    assert_eq!(serde_json::from_str::<Head>(&s).unwrap(), h);
    let d = Diff::new(
        crosswire_core::ChangeOp::Replace,
        crosswire_core::Span::new(1, 3),
        bytes::Bytes::from_static(b"ab"),
        bytes::Bytes::from_static(b"c"),
        crosswire_core::ChangeReason::HopByHop,
    );
    let s = serde_json::to_string(&d).unwrap();
    assert_eq!(serde_json::from_str::<Diff>(&s).unwrap(), d);
}
