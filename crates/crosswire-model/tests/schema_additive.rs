//! Fields added after the first schema: old JSON still reads, new JSON
//! round-trips.
mod common;
use bytes::Bytes;
use common::*;

#[test]
fn old_inline_body_json_still_deserialises() {
    let b: Body = serde_json::from_str(r#"{"type":"inline","bytes":"aGk="}"#).unwrap();
    let Body::Inline {
        bytes,
        truncated_at,
        capture_dropped,
        relay,
    } = b
    else {
        panic!("inline expected, got {b:?}")
    };
    assert_eq!(&bytes[..], b"hi");
    assert_eq!(truncated_at, None);
    assert!(!capture_dropped);
    assert_eq!(relay, None);
    assert_eq!(
        Body::inline(Bytes::from_static(b"hi")),
        serde_json::from_str(r#"{"type":"inline","bytes":"aGk="}"#).unwrap()
    );
}

#[test]
fn old_blob_body_json_has_no_relay() {
    let json = format!(
        r#"{{"type":"blob","hash":"{}","len":5}}"#,
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
    );
    let b: Body = serde_json::from_str(&json).unwrap();
    let Body::Blob { relay, len, .. } = b else {
        panic!("blob expected")
    };
    assert_eq!(len, 5);
    assert_eq!(relay, None);
}

#[test]
fn old_overridden_json_still_deserialises() {
    let c: ChainResult = serde_json::from_str(r#"{"type":"overridden"}"#).unwrap();
    assert_eq!(
        c,
        ChainResult::Overridden {
            reason: String::new()
        }
    );
    let c = ChainResult::Overridden {
        reason: "UnknownIssuer".into(),
    };
    let s = serde_json::to_string(&c).unwrap();
    assert_eq!(serde_json::from_str::<ChainResult>(&s).unwrap(), c);
}

#[test]
fn message_without_interim_deserialises_empty() {
    let raw = parse_resp(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
    let m = Message::http1_raw(&raw, Body::None, conn("x", 80));
    let mut v = serde_json::to_value(&m).unwrap();
    // An old writer never wrote the field.
    v.as_object_mut().unwrap().remove("interim");
    let back: Message = serde_json::from_value(v).unwrap();
    assert!(back.interim().is_empty());
    assert_eq!(back, m);
}

#[test]
fn interim_heads_round_trip() {
    let raw = parse_resp(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
    let interim = vec![
        Bytes::from_static(b"HTTP/1.1 100 Continue\r\n\r\n"),
        Bytes::from_static(b"HTTP/1.1 103 Early Hints\r\nLink: </a.css>\r\n\r\n"),
    ];
    let m = Message::http1_raw(&raw, Body::None, conn("x", 80)).with_interim(interim.clone());
    assert_eq!(m.interim(), &interim[..]);
    let s = serde_json::to_string(&m).unwrap();
    assert!(
        s.contains("\"interim\":[\"SFRUUC8xLjEgMTAwIENvbnRpbnVlDQoNCg==\""),
        "{s}"
    );
    assert_eq!(serde_json::from_str::<Message>(&s).unwrap(), m);
}

#[test]
fn capture_round_trips() {
    let relay = Capture {
        len: 70_000,
        sha256: [0xab; 32],
    };
    let body = Body::Inline {
        bytes: Bytes::from(vec![b'a'; 1024]),
        truncated_at: Some(1024),
        capture_dropped: false,
        relay: Some(relay),
    };
    let s = serde_json::to_string(&body).unwrap();
    assert!(s.contains("\"truncated_at\":1024"), "{s}");
    assert!(s.contains("\"len\":70000"), "{s}");
    assert_eq!(serde_json::from_str::<Body>(&s).unwrap(), body);

    let blob = Body::Blob {
        hash: [1; 32],
        len: 1024,
        truncated_at: Some(1024),
        capture_dropped: true,
        relay: Some(relay),
    };
    let s = serde_json::to_string(&blob).unwrap();
    assert_eq!(serde_json::from_str::<Body>(&s).unwrap(), blob);
}

#[test]
fn peer_addr_is_separate_from_the_configured_host() {
    let c = ConnectionInfo::new("example.test", 443).with_peer_addr("203.0.113.7");
    assert_eq!(c.target_addr(), "example.test");
    assert_eq!(c.peer_addr(), Some("203.0.113.7"));
    let s = serde_json::to_string(&c).unwrap();
    assert_eq!(serde_json::from_str::<ConnectionInfo>(&s).unwrap(), c);
    // Old JSON without the field.
    let old: ConnectionInfo =
        serde_json::from_str(r#"{"target_addr":"example.test","port":443}"#).unwrap();
    assert_eq!(old.peer_addr(), None);
}
