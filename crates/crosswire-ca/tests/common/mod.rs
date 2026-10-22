//! Shared helpers for the CA integration tests.
#![allow(dead_code)]

use std::sync::Arc;

use crosswire_ca::Ca;
use rustls::client::danger::ServerCertVerifier;
use rustls::client::WebPkiServerVerifier;
use rustls::crypto::ring::default_provider;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::sign::CertifiedKey;
use rustls::RootCertStore;

/// True when the leaf in `ck` verifies against `ca` for `name` (webpki chain and name checks).
pub fn verifies(ca: &Ca, ck: &CertifiedKey, name: &str) -> bool {
    let mut roots = RootCertStore::empty();
    roots.add(CertificateDer::from(ca.cert_der())).unwrap();
    let verifier =
        WebPkiServerVerifier::builder_with_provider(Arc::new(roots), Arc::new(default_provider()))
            .build()
            .unwrap();
    let server_name = ServerName::try_from(name.to_string()).unwrap();
    verifier
        .verify_server_cert(
            &ck.cert[0],
            &ck.cert[1..],
            &server_name,
            &[],
            UnixTime::now(),
        )
        .is_ok()
}

/// Panics unless the leaf verifies.
pub fn assert_verifies(ca: &Ca, ck: &CertifiedKey, name: &str) {
    assert!(verifies(ca, ck, name), "leaf must verify for {name}");
}
