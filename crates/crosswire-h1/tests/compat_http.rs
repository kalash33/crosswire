//! Tests for the lossy `http` crate conversion.
#![cfg(feature = "http")]

use crosswire_h1::{HttpConvertError, Parser, ParserConfig, Progress, RawMessage, ResponseContext};

fn req(bytes: &[u8]) -> RawMessage {
    let mut p = Parser::request(&ParserConfig::default());
    match p.feed(bytes).unwrap().event {
        Progress::Head(m) => m,
        _ => panic!("no head"),
    }
}

fn resp(bytes: &[u8]) -> RawMessage {
    let mut p = Parser::response(
        &ParserConfig::default(),
        ResponseContext::for_method(b"GET"),
    );
    match p.feed(bytes).unwrap().event {
        Progress::Head(m) => m,
        _ => panic!("no head"),
    }
}

#[test]
fn clean_request() {
    let m = req(b"GET /a?b=1 HTTP/1.1\r\nHost: x.io\r\nX-A: 1\r\n\r\n");
    let r = http::Request::<()>::try_from(&m).unwrap();
    assert_eq!(r.method(), http::Method::GET);
    assert_eq!(r.uri(), "/a?b=1");
    assert_eq!(r.version(), http::Version::HTTP_11);
    assert_eq!(r.headers()["host"], "x.io");
    assert_eq!(r.headers()["x-a"], "1");
    assert_eq!(r.headers().len(), 2);
}

#[test]
fn obs_fold_joined_with_single_space() {
    let m = req(b"GET / HTTP/1.1\r\nHost: x\r\nX-A: 1\r\n 2\r\n\r\n");
    let r = http::Request::<()>::try_from(&m).unwrap();
    assert_eq!(r.headers()["x-a"], "1 2");
}

#[test]
fn obs_fold_bare_lf_and_tabs() {
    let m = req(b"GET / HTTP/1.1\nHost: x\nX-A: 1\n \t 2\n\n");
    let r = http::Request::<()>::try_from(&m).unwrap();
    assert_eq!(r.headers()["x-a"], "1 2");
}

#[test]
fn space_before_colon_host() {
    let m = req(b"GET / HTTP/1.1\r\nHost : x\r\n\r\n");
    let r = http::Request::<()>::try_from(&m).unwrap();
    assert_eq!(r.headers()["host"], "x");
}

#[test]
fn duplicate_host() {
    let m = req(b"GET / HTTP/1.1\r\nHost: a\r\nhost: b\r\n\r\n");
    assert_eq!(
        http::Request::<()>::try_from(&m).unwrap_err(),
        HttpConvertError::DuplicateHost
    );
}

#[test]
fn invalid_header_name_span() {
    let raw: &[u8] = b"GET / HTTP/1.1\r\nHost: x\r\nX(A: 1\r\n\r\n";
    let m = req(raw);
    let HttpConvertError::InvalidHeaderName { span } =
        http::Request::<()>::try_from(&m).unwrap_err()
    else {
        panic!("wrong error")
    };
    assert_eq!(&m.bytes()[span.range()], b"X(A");
}

#[test]
fn invalid_header_value_span() {
    let m = req(b"GET / HTTP/1.1\r\nHost: x\r\nX-A: a\x7fb\r\n\r\n");
    let HttpConvertError::InvalidHeaderValue { span } =
        http::Request::<()>::try_from(&m).unwrap_err()
    else {
        panic!("wrong error")
    };
    assert_eq!(&m.bytes()[span.range()], b"a\x7fb");
}

#[test]
fn invalid_method_span() {
    let m = req(b"G(T / HTTP/1.1\r\nHost: x\r\n\r\n");
    let HttpConvertError::InvalidMethod { span } = http::Request::<()>::try_from(&m).unwrap_err()
    else {
        panic!("wrong error")
    };
    assert_eq!(&m.bytes()[span.range()], b"G(T");
}

#[test]
fn invalid_uri_span() {
    let m = req(b"GET /a<b HTTP/1.1\r\nHost: x\r\n\r\n");
    let HttpConvertError::InvalidUri { span } = http::Request::<()>::try_from(&m).unwrap_err()
    else {
        panic!("wrong error")
    };
    assert_eq!(&m.bytes()[span.range()], b"/a<b");
}

#[test]
fn invalid_version() {
    let m = req(b"GET / HTTP/1.7\r\nHost: x\r\n\r\n");
    let HttpConvertError::InvalidVersion { span } = http::Request::<()>::try_from(&m).unwrap_err()
    else {
        panic!("wrong error")
    };
    assert_eq!(&m.bytes()[span.range()], b"HTTP/1.7");
}

#[test]
fn http_10_maps() {
    let m = req(b"GET / HTTP/1.0\r\n\r\n");
    let r = http::Request::<()>::try_from(&m).unwrap();
    assert_eq!(r.version(), http::Version::HTTP_10);
}

#[test]
fn response_into_request_is_wrong_kind() {
    let m = resp(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
    assert_eq!(
        http::Request::<()>::try_from(&m).unwrap_err(),
        HttpConvertError::WrongKind
    );
}

#[test]
fn request_into_response_is_wrong_kind() {
    let m = req(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n");
    assert_eq!(
        http::Response::<()>::try_from(&m).unwrap_err(),
        HttpConvertError::WrongKind
    );
}

#[test]
fn clean_response() {
    let m = resp(
        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nSet-Cookie: a\r\nSet-Cookie: b\r\n\r\n",
    );
    let r = http::Response::<()>::try_from(&m).unwrap();
    assert_eq!(r.status(), 404);
    assert_eq!(r.version(), http::Version::HTTP_11);
    assert_eq!(r.headers().get_all("set-cookie").iter().count(), 2);
}

#[test]
fn response_invalid_version_and_status() {
    let m = resp(b"HTTP/2.0 200 OK\r\n\r\n");
    assert!(matches!(
        http::Response::<()>::try_from(&m).unwrap_err(),
        HttpConvertError::InvalidVersion { .. }
    ));
    // Status outside http's 100..=999 range (three digits, but 099).
    let m = resp(b"HTTP/1.1 099 Low\r\n\r\n");
    let e = http::Response::<()>::try_from(&m);
    if let Err(HttpConvertError::InvalidStatus { span }) = e {
        assert_eq!(&m.bytes()[span.range()], b"099");
    } else {
        panic!("expected InvalidStatus, got {e:?}");
    }
}

#[test]
fn inline_body_ignored() {
    let m = resp(b"HTTP/1.1 200 OK\r\n\r\n").with_inline_body(bytes::Bytes::from_static(b"hi"));
    assert!(http::Response::<()>::try_from(&m).is_ok());
}

#[test]
fn error_display() {
    assert!(!HttpConvertError::DuplicateHost.to_string().is_empty());
    let e: &dyn std::error::Error = &HttpConvertError::WrongKind;
    assert!(!e.to_string().is_empty());
}
