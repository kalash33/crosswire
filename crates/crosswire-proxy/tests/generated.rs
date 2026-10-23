//! Generated responses and config validation.

use crosswire_model::{ConnectionInfo, Fidelity, Head};
use crosswire_proxy::__test::generated;
use crosswire_proxy::{ProxyAuth, ProxyConfig, ProxyError};

#[test]
fn generated_502_is_parseable_and_marked() {
    let body = "Crosswire could not connect to a.test:80: refused";
    let (bytes, msg) = generated(502, "Bad Gateway", body, ConnectionInfo::new("a.test", 80));
    assert_eq!(msg.fidelity(), Fidelity::Generated);
    let Head::Http1Raw { bytes: head, .. } = msg.head() else {
        panic!("expected raw head");
    };
    let head_text = std::str::from_utf8(head).unwrap();
    assert!(head_text.starts_with("HTTP/1.1 502 Bad Gateway\r\n"));
    assert!(head_text.contains(&format!("Content-Length: {}\r\n", body.len())));
    assert!(head_text.contains("Connection: close\r\n"));
    assert!(head_text.contains("X-Crosswire-Generated: 1\r\n"));
    assert!(bytes.ends_with(body.as_bytes()));
    assert_eq!(bytes.len(), head.len() + body.len());
}

fn cfg(bind: &str) -> ProxyConfig {
    ProxyConfig::new(bind.parse().unwrap())
}

#[test]
fn config_refuses_non_loopback_without_flag() {
    assert!(matches!(
        cfg("0.0.0.0:8080").validate(),
        Err(ProxyError::NonLoopbackBind)
    ));
    assert!(cfg("127.0.0.1:8080").validate().is_ok());
    assert!(cfg("[::1]:8080").validate().is_ok());
}

#[test]
fn config_requires_auth_on_non_loopback() {
    let mut c = cfg("0.0.0.0:8080");
    c.allow_non_loopback = true;
    assert!(matches!(c.validate(), Err(ProxyError::AuthRequired)));
    c.proxy_auth = Some(ProxyAuth::new("u", "p"));
    assert!(c.validate().is_ok());
}

#[test]
fn config_rejects_bad_passthrough_entry() {
    let mut c = cfg("127.0.0.1:0");
    c.passthrough = vec!["example.com".into(), ".corp.test".into()];
    assert!(c.validate().is_ok());
    for bad in [
        "not a host!",
        "",
        ".",
        "-a.test",
        "a..test",
        &"a".repeat(254),
    ] {
        c.passthrough = vec![bad.to_string()];
        assert!(
            matches!(c.validate(), Err(ProxyError::Config(_))),
            "{bad:?}"
        );
    }
}
