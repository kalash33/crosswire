//! Leaf issuance: validation, SANs, caching, rate limit, validity bounds.

mod common;

use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;

use common::{assert_verifies, verifies};
use crosswire_ca::{Ca, CaError, IssuerLimits, LeafIssuer};
use rustls::pki_types::ServerName;
use x509_parser::extensions::{GeneralName, ParsedExtension};
use x509_parser::prelude::*;

fn issuer(limits: IssuerLimits) -> (Ca, LeafIssuer) {
    let ca = Ca::generate("test").unwrap();
    let issuer = LeafIssuer::new(Arc::new(ca.clone()), limits);
    (ca, issuer)
}

#[test]
fn leaf_validates_against_ca_with_webpki() {
    let (ca, issuer) = issuer(IssuerLimits::default());
    let ck = issuer
        .issue(&ServerName::try_from("example.test").unwrap())
        .unwrap();
    assert_verifies(&ca, &ck, "example.test");
    assert!(!verifies(&ca, &ck, "other.test"));
    // A different CA must not validate it.
    let other = Ca::generate("other").unwrap();
    assert!(!verifies(&other, &ck, "example.test"));
}

#[test]
fn ip_literal_gets_ip_san() {
    let (ca, issuer) = issuer(IssuerLimits::default());
    let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
    let ck = issuer.issue(&ServerName::IpAddress(ip.into())).unwrap();
    let (_, cert) = X509Certificate::from_der(&ck.cert[0]).unwrap();
    let san = cert
        .extensions()
        .iter()
        .find_map(|e| match e.parsed_extension() {
            ParsedExtension::SubjectAlternativeName(s) => Some(s.clone()),
            _ => None,
        })
        .expect("SAN present");
    assert_eq!(san.general_names.len(), 1);
    match &san.general_names[0] {
        GeneralName::IPAddress(b) => assert_eq!(*b, &[127, 0, 0, 1]),
        other => panic!("expected IP SAN, got {other:?}"),
    }
    assert_verifies_ip(&ca, &ck);
}

fn assert_verifies_ip(ca: &Ca, ck: &rustls::sign::CertifiedKey) {
    use rustls::client::danger::ServerCertVerifier;
    use rustls::client::WebPkiServerVerifier;
    use rustls::pki_types::{CertificateDer, UnixTime};
    let mut roots = rustls::RootCertStore::empty();
    roots.add(CertificateDer::from(ca.cert_der())).unwrap();
    let v = WebPkiServerVerifier::builder_with_provider(
        Arc::new(roots),
        Arc::new(rustls::crypto::ring::default_provider()),
    )
    .build()
    .unwrap();
    let name = ServerName::IpAddress(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)).into());
    v.verify_server_cert(&ck.cert[0], &ck.cert[1..], &name, &[], UnixTime::now())
        .unwrap();
}

#[test]
fn cache_hit_returns_same_arc() {
    let (_ca, issuer) = issuer(IssuerLimits::default());
    let name = ServerName::try_from("example.test").unwrap();
    let a = issuer.issue(&name).unwrap();
    let b = issuer.issue(&name).unwrap();
    assert!(Arc::ptr_eq(&a, &b));
    assert_eq!(issuer.cached_len(), 1);
    // Host matching is case-insensitive.
    let c = issuer
        .issue(&ServerName::try_from("EXAMPLE.test").unwrap())
        .unwrap();
    assert!(Arc::ptr_eq(&a, &c));
}

#[test]
fn cache_is_bounded() {
    let (_ca, issuer) = issuer(IssuerLimits {
        cache_entries: 2,
        mint_rate_per_sec: 1000,
    });
    for h in ["a.test", "b.test", "c.test"] {
        issuer.issue(&ServerName::try_from(h).unwrap()).unwrap();
    }
    assert_eq!(issuer.cached_len(), 2);
}

#[test]
fn mint_rate_limit() {
    let (_ca, issuer) = issuer(IssuerLimits {
        cache_entries: 16,
        mint_rate_per_sec: 2,
    });
    issuer
        .issue(&ServerName::try_from("a.test").unwrap())
        .unwrap();
    issuer
        .issue(&ServerName::try_from("b.test").unwrap())
        .unwrap();
    let err = issuer
        .issue(&ServerName::try_from("c.test").unwrap())
        .unwrap_err();
    assert!(matches!(err, CaError::RateLimited));
    // Cache hits are not rate limited.
    issuer
        .issue(&ServerName::try_from("a.test").unwrap())
        .unwrap();
}

#[test]
fn leaf_validity_bounds() {
    let (ca, issuer) = issuer(IssuerLimits::default());
    let ck = issuer
        .issue(&ServerName::try_from("example.test").unwrap())
        .unwrap();
    let (_, leaf) = X509Certificate::from_der(&ck.cert[0]).unwrap();
    let ca_der = ca.cert_der();
    let (_, ca_cert) = X509Certificate::from_der(&ca_der).unwrap();
    let now = ::time::OffsetDateTime::now_utc().unix_timestamp();
    let day = 86_400;
    let nb = leaf.validity().not_before.timestamp();
    let na = leaf.validity().not_after.timestamp();
    assert!(nb <= now - 23 * 3600, "notBefore must be about a day back");
    assert!(na <= now + 398 * day + 5, "notAfter within 398 days");
    assert!(na > now + 397 * day);
    assert!(na < ca_cert.validity().not_after.timestamp());
    // CA: roughly ten years, begins a day back.
    let ca_na = ca_cert.validity().not_after.timestamp();
    assert!(ca_na > now + 3649 * day && ca_na <= now + 3651 * day);
    assert!(ca_cert.validity().not_before.timestamp() <= now - 23 * 3600);
}

#[test]
fn leaf_extensions_and_ca_profile() {
    let (ca, issuer) = issuer(IssuerLimits::default());
    let ck = issuer
        .issue(&ServerName::try_from("example.test").unwrap())
        .unwrap();
    let (_, leaf) = X509Certificate::from_der(&ck.cert[0]).unwrap();
    let eku = leaf.extended_key_usage().unwrap().unwrap().value;
    assert!(eku.server_auth);
    assert!(leaf
        .basic_constraints()
        .unwrap()
        .is_none_or(|b| !b.value.ca));
    assert_eq!(ck.cert.len(), 2, "chain is leaf + CA");
    assert_eq!(ck.cert[1].as_ref(), ca.cert_der().as_slice());

    let ca_der = ca.cert_der();
    let (_, c) = X509Certificate::from_der(&ca_der).unwrap();
    assert!(c.basic_constraints().unwrap().unwrap().value.ca);
    let ku = c.key_usage().unwrap().unwrap().value;
    assert!(ku.key_cert_sign() && ku.crl_sign());
    let cn = c
        .subject()
        .iter_common_name()
        .next()
        .unwrap()
        .as_str()
        .unwrap();
    assert_eq!(cn, "Crosswire CA test");
}

#[test]
fn leaves_have_distinct_serials_and_one_key() {
    let (_ca, issuer) = issuer(IssuerLimits::default());
    let a = issuer
        .issue(&ServerName::try_from("a.test").unwrap())
        .unwrap();
    let b = issuer
        .issue(&ServerName::try_from("b.test").unwrap())
        .unwrap();
    let (_, ca_) = X509Certificate::from_der(&a.cert[0]).unwrap();
    let (_, cb) = X509Certificate::from_der(&b.cert[0]).unwrap();
    assert_ne!(ca_.raw_serial(), cb.raw_serial());
    assert_eq!(
        ca_.public_key().raw,
        cb.public_key().raw,
        "one leaf keypair per issuer"
    );
}

#[test]
fn spki_pin_is_base64_sha256() {
    let ca = Ca::generate("test").unwrap();
    let pin = ca.spki_sha256_base64();
    assert_eq!(pin.len(), 44);
    assert!(pin.ends_with('='));
    assert_eq!(
        pin,
        Ca::from_pem(&ca.cert_pem(), &ca.key_pem())
            .unwrap()
            .spki_sha256_base64()
    );
}

#[test]
fn issuer_is_send_sync() {
    fn assert<T: Send + Sync>() {}
    assert::<LeafIssuer>();
    assert::<Ca>();
}
