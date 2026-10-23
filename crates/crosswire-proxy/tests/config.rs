//! Config validation.

use crosswire_proxy::{ProxyAuth, ProxyConfig, ProxyError};

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
    c.proxy_auth = Some(ProxyAuth::new("u", "p").unwrap());
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

#[test]
fn config_rejects_bad_accept_invalid_cert_entry() {
    let mut c = cfg("127.0.0.1:0");
    c.accept_invalid_cert = vec!["bad.test".into()];
    assert!(c.validate().is_ok());
    for bad in ["not a host!", "", ".suffix.test", "a..test"] {
        c.accept_invalid_cert = vec![bad.to_string()];
        assert!(
            matches!(c.validate(), Err(ProxyError::Config(_))),
            "{bad:?}"
        );
    }
}

#[test]
fn proxy_auth_rejects_empty_parts() {
    assert!(matches!(
        ProxyAuth::new("", "p"),
        Err(ProxyError::Config(_))
    ));
    assert!(matches!(
        ProxyAuth::new("u", ""),
        Err(ProxyError::Config(_))
    ));
    assert!(ProxyAuth::new("u", "p").is_ok());
}
