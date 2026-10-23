//! Config validation.

mod common;

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

#[tokio::test]
async fn from_listener_serves_on_the_given_listener() {
    use std::sync::Arc;

    use crosswire_ca::Ca;
    use crosswire_proxy::{NoopHandler, Proxy};

    let ca = Arc::new(Ca::generate("proxy-test").unwrap());
    let sink = Arc::new(common::TestSink::default());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    // `cfg.bind` is overridden by the listener's address.
    let proxy = Proxy::from_listener(
        cfg("127.0.0.1:1"),
        listener,
        common::deps(&ca, sink, Arc::new(NoopHandler)),
    )
    .unwrap();
    assert_eq!(proxy.local_addr(), addr);

    // A non-loopback listener is validated like a non-loopback bind.
    let wide = tokio::net::TcpListener::bind("0.0.0.0:0").await.unwrap();
    let sink = Arc::new(common::TestSink::default());
    let refused = Proxy::from_listener(
        cfg("127.0.0.1:0"),
        wide,
        common::deps(&ca, sink, Arc::new(NoopHandler)),
    );
    assert!(matches!(refused, Err(ProxyError::NonLoopbackBind)));
}
