//! Transport over plain TCP and TLS, with the gate in front.
mod common;

use std::net::SocketAddr;
use std::sync::Arc;

use crosswire_client::{
    ClientError, DestinationGate, GateRefusal, RustlsBackend, Target, Transport, VerifyMode,
};
use crosswire_model::ChainResult;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn gate_allowing(addr: SocketAddr) -> Arc<DestinationGate> {
    let mut g = DestinationGate::new(vec![]);
    g.allow(&addr.ip().to_string(), addr.port());
    Arc::new(g)
}

fn host(addr: SocketAddr) -> String {
    addr.ip().to_string()
}

#[tokio::test]
async fn verifies_ca_issued_leaf_and_refuses_unknown_chain() {
    let (ca, addr) = common::spawn_tls_echo_with_ca().await;
    let t = Transport::new(
        Arc::new(RustlsBackend::with_extra_roots(vec![ca.cert_der()])),
        gate_allowing(addr),
    );
    let c = t
        .connect(&Target::tls(
            &host(addr),
            addr.port(),
            "example.test",
            VerifyMode::WebPki,
        ))
        .await
        .unwrap();
    assert!(matches!(
        c.info().upstream_chain_result(),
        ChainResult::Valid
    ));
    assert!(!c.info().cert_override());

    let bad_addr = common::spawn_tls_echo_self_signed().await;
    let t = Transport::new(
        Arc::new(RustlsBackend::with_extra_roots(vec![ca.cert_der()])),
        gate_allowing(bad_addr),
    );
    let err = t
        .connect(&Target::tls(
            &host(bad_addr),
            bad_addr.port(),
            "bad.test",
            VerifyMode::WebPki,
        ))
        .await
        .unwrap_err();
    assert!(matches!(err, ClientError::Tls(_)), "{err:?}");
}

#[tokio::test]
async fn accept_invalid_overrides_and_records_the_reason() {
    let bad_addr = common::spawn_tls_echo_self_signed().await;
    let t = Transport::new(
        Arc::new(RustlsBackend::webpki_roots()),
        gate_allowing(bad_addr),
    );
    let c = t
        .connect(&Target::tls(
            &host(bad_addr),
            bad_addr.port(),
            "bad.test",
            VerifyMode::AcceptInvalid,
        ))
        .await
        .unwrap();
    assert!(matches!(
        c.info().upstream_chain_result(),
        ChainResult::Overridden
    ));
    assert!(c.info().cert_override());
    let reason = c.chain_error().expect("override reason recorded");
    assert!(reason.contains("UnknownIssuer"), "{reason}");
}

#[tokio::test]
async fn accept_invalid_reports_valid_when_chain_was_fine() {
    let (ca, addr) = common::spawn_tls_echo_with_ca().await;
    let t = Transport::new(
        Arc::new(RustlsBackend::with_extra_roots(vec![ca.cert_der()])),
        gate_allowing(addr),
    );
    let c = t
        .connect(&Target::tls(
            &host(addr),
            addr.port(),
            "example.test",
            VerifyMode::AcceptInvalid,
        ))
        .await
        .unwrap();
    assert!(matches!(
        c.info().upstream_chain_result(),
        ChainResult::Valid
    ));
    assert!(!c.info().cert_override());
    assert!(c.chain_error().is_none());
}

#[tokio::test]
async fn wrong_name_is_refused_under_webpki() {
    let (ca, addr) = common::spawn_tls_echo_with_ca().await;
    let t = Transport::new(
        Arc::new(RustlsBackend::with_extra_roots(vec![ca.cert_der()])),
        gate_allowing(addr),
    );
    let err = t
        .connect(&Target::tls(
            &host(addr),
            addr.port(),
            "other.test",
            VerifyMode::WebPki,
        ))
        .await
        .unwrap_err();
    assert!(matches!(err, ClientError::Tls(_)), "{err:?}");
}

#[tokio::test]
async fn records_alpn_version_cipher() {
    let (ca, addr) = common::spawn_tls_echo_with_ca().await;
    let t = Transport::new(
        Arc::new(RustlsBackend::with_extra_roots(vec![ca.cert_der()])),
        gate_allowing(addr),
    );
    let c = t
        .connect(&Target::tls(
            &host(addr),
            addr.port(),
            "example.test",
            VerifyMode::WebPki,
        ))
        .await
        .unwrap();
    let i = c.info();
    assert_eq!(i.alpn(), Some("http/1.1"));
    assert_eq!(i.tls_version(), Some("TLSv1_3"));
    assert!(!i.cipher().unwrap().is_empty());
    assert_eq!(i.sni(), Some("example.test"));
    assert_eq!(i.target_addr(), "127.0.0.1");
    assert_eq!(i.port(), addr.port());
    assert!(!i.reused());
    // Bytes flow through the boxed connection.
    let (mut io, info, _, _) = c.into_parts();
    io.write_all(b"ping").await.unwrap();
    let mut b = [0u8; 4];
    io.read_exact(&mut b).await.unwrap();
    assert_eq!(&b, b"ping");
    assert_eq!(info.sni(), Some("example.test"));
}

#[tokio::test]
async fn plain_tcp_connection_has_not_tls() {
    let addr = common::spawn_plain_echo().await;
    let t = Transport::new(Arc::new(RustlsBackend::webpki_roots()), gate_allowing(addr));
    let c = t
        .connect(&Target::plain(&host(addr), addr.port()))
        .await
        .unwrap();
    assert!(matches!(
        c.info().upstream_chain_result(),
        ChainResult::NotTls
    ));
    assert_eq!(c.tls_ms(), 0);
    assert!(c.info().tls_version().is_none());
}

#[tokio::test]
async fn conn_ids_are_unique() {
    let a = common::spawn_plain_echo().await;
    let b = common::spawn_plain_echo().await;
    let mut g = DestinationGate::new(vec![]);
    g.allow("127.0.0.1", a.port());
    g.allow("127.0.0.1", b.port());
    let t = Transport::new(Arc::new(RustlsBackend::webpki_roots()), Arc::new(g));
    let c1 = t
        .connect(&Target::plain("127.0.0.1", a.port()))
        .await
        .unwrap();
    let c2 = t
        .connect(&Target::plain("127.0.0.1", b.port()))
        .await
        .unwrap();
    assert_ne!(c1.info().conn_id(), c2.info().conn_id());
}

#[tokio::test]
async fn gate_applies_to_hostnames() {
    let addr = common::spawn_plain_echo().await;
    let t = Transport::new(
        Arc::new(RustlsBackend::webpki_roots()),
        Arc::new(DestinationGate::new(vec![])),
    );
    let err = t
        .connect(&Target::plain("localhost", addr.port()))
        .await
        .unwrap_err();
    assert!(
        matches!(err, ClientError::Refused(GateRefusal::Loopback(_))),
        "{err:?}"
    );
}

#[tokio::test]
async fn gate_applies_to_alternate_spellings() {
    let addr = common::spawn_plain_echo().await;
    let t = Transport::new(
        Arc::new(RustlsBackend::webpki_roots()),
        Arc::new(DestinationGate::new(vec![])),
    );
    for h in [
        "127.0.0.1",
        "[::1]",
        "::1",
        "127.1",
        "2130706433",
        "0x7f.0.0.1",
        "::ffff:127.0.0.1",
        "169.254.169.254",
        "LOCALHOST",
        "localhost.",
    ] {
        let r = t.connect(&Target::plain(h, addr.port())).await;
        // A spelling the platform resolver accepts must be refused by the
        // gate; only one it rejects may end in a DNS error (the transport
        // may still normalise it, e.g. strip brackets, and refuse it). Never
        // connected.
        let resolves = std::net::ToSocketAddrs::to_socket_addrs(&(h, addr.port()))
            .is_ok_and(|mut a| a.next().is_some());
        if resolves {
            assert!(
                matches!(r, Err(ClientError::Refused(_))),
                "{h} resolves, so the gate must refuse it: {:?}",
                r.map(|_| ())
            );
        } else {
            assert!(
                matches!(r, Err(ClientError::Refused(_) | ClientError::Dns(_))),
                "{h}: {:?}",
                r.map(|_| ())
            );
        }
    }
}

#[tokio::test]
async fn own_listener_is_refused_even_when_allowlisted() {
    let addr = common::spawn_plain_echo().await;
    let mut g = DestinationGate::new(vec![addr.port()]);
    g.allow("127.0.0.1", addr.port());
    let t = Transport::new(Arc::new(RustlsBackend::webpki_roots()), Arc::new(g));
    let err = t
        .connect(&Target::plain("127.0.0.1", addr.port()))
        .await
        .unwrap_err();
    assert!(
        matches!(err, ClientError::Refused(GateRefusal::OwnListener(_))),
        "{err:?}"
    );
}

#[tokio::test]
async fn invalid_sni_is_an_error_not_a_fallback() {
    let addr = common::spawn_plain_echo().await;
    let t = Transport::new(Arc::new(RustlsBackend::webpki_roots()), gate_allowing(addr));
    let err = t
        .connect(&Target::tls(
            &host(addr),
            addr.port(),
            "not a host name",
            VerifyMode::WebPki,
        ))
        .await
        .unwrap_err();
    assert!(matches!(err, ClientError::Tls(_)), "{err:?}");
}

#[tokio::test]
async fn bracketed_ipv6_host_matches_unbracketed_allowlist_entry() {
    let Ok(l) = tokio::net::TcpListener::bind("[::1]:0").await else {
        return; // no IPv6 loopback on this machine
    };
    let port = l.local_addr().unwrap().port();
    tokio::spawn(async move {
        let _ = l.accept().await;
    });
    let mut g = DestinationGate::new(vec![]);
    g.allow("::1", port);
    let t = Transport::new(Arc::new(RustlsBackend::webpki_roots()), Arc::new(g));
    let c = t.connect(&Target::plain("[::1]", port)).await.unwrap();
    assert_eq!(c.info().port(), port);
}
