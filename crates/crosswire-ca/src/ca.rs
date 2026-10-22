//! The local certificate authority.

use std::sync::Arc;

use rcgen::{
    BasicConstraints, CertificateParams, DnType, IsCa, Issuer, KeyPair, KeyUsagePurpose,
    SerialNumber, PKCS_ECDSA_P256_SHA256,
};
use sha2::{Digest, Sha256};
use time::{Duration, OffsetDateTime};

use crate::error::CaError;
use crate::keystore::KeyStore;

/// CA validity: ten years.
const CA_VALIDITY_DAYS: i64 = 3650;

pub(crate) struct CaInner {
    cert_pem: String,
    cert_der: Vec<u8>,
    key_pem: String,
    pub(crate) issuer: Issuer<'static, KeyPair>,
    pub(crate) not_after: OffsetDateTime,
    spki_pin: String,
}

/// A local certificate authority: a P-256 key and its self-signed certificate.
///
/// Cloning is cheap (shared state). The private key is reachable only through
/// [`Ca::key_pem`], which exists for the key store; never log or export it.
#[derive(Clone)]
pub struct Ca {
    pub(crate) inner: Arc<CaInner>,
}

impl std::fmt::Debug for Ca {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ca")
            .field("spki_sha256", &self.inner.spki_pin)
            .finish_non_exhaustive()
    }
}

fn gen_err(e: impl std::fmt::Display) -> CaError {
    CaError::Generate(e.to_string())
}

fn pem_err(e: impl std::fmt::Display) -> CaError {
    CaError::Pem(e.to_string())
}

/// A random positive 127-bit serial number.
pub(crate) fn random_serial() -> Result<SerialNumber, CaError> {
    let mut b = [0u8; 16];
    rustls::crypto::ring::default_provider()
        .secure_random
        .fill(&mut b)
        .map_err(|_| CaError::Generate("system randomness unavailable".into()))?;
    b[0] = (b[0] & 0x7f) | 0x40;
    Ok(SerialNumber::from_slice(&b))
}

impl Ca {
    /// Generate a new CA named `Crosswire CA <subject_suffix>`: P-256, `CA:TRUE`,
    /// `keyCertSign | cRLSign`, valid for ten years starting one day ago.
    pub fn generate(subject_suffix: &str) -> Result<Ca, CaError> {
        let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).map_err(gen_err)?;
        let mut params = CertificateParams::default();
        let now = OffsetDateTime::now_utc();
        params.not_before = now - Duration::days(1);
        params.not_after = now + Duration::days(CA_VALIDITY_DAYS);
        params.serial_number = Some(random_serial()?);
        params.distinguished_name = rcgen::DistinguishedName::new();
        params
            .distinguished_name
            .push(DnType::CommonName, format!("Crosswire CA {subject_suffix}"));
        params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        let cert = params.self_signed(&key).map_err(gen_err)?;
        Self::build(cert.pem(), cert.der().to_vec(), key)
    }

    /// Load a CA from PEM text. Fails if the key does not match the certificate.
    pub fn from_pem(cert_pem: &str, key_pem: &str) -> Result<Ca, CaError> {
        let pem = pem_decode(cert_pem)?;
        let key = KeyPair::from_pem(key_pem).map_err(pem_err)?;
        Self::build(cert_pem.to_owned(), pem, key)
    }

    fn build(cert_pem: String, cert_der: Vec<u8>, key: KeyPair) -> Result<Ca, CaError> {
        let (_, x509) = x509_parser::parse_x509_certificate(&cert_der).map_err(pem_err)?;
        if x509.subject_pki.subject_public_key.data.as_ref() != key.public_key_raw() {
            return Err(CaError::Pem("key does not match certificate".into()));
        }
        if !x509.is_ca() {
            return Err(CaError::Pem("certificate is not a CA".into()));
        }
        let not_after = OffsetDateTime::from_unix_timestamp(x509.validity().not_after.timestamp())
            .map_err(pem_err)?;
        let spki_pin = base64(&Sha256::digest(x509.subject_pki.raw));
        let key_pem = key.serialize_pem();
        let issuer = Issuer::from_ca_cert_der(&cert_der.clone().into(), key).map_err(pem_err)?;
        Ok(Ca {
            inner: Arc::new(CaInner {
                cert_pem,
                cert_der,
                key_pem,
                issuer,
                not_after,
                spki_pin,
            }),
        })
    }

    /// Load the CA from `store`, or generate and save a new one when it is empty.
    pub fn load_or_generate(store: &dyn KeyStore, subject_suffix: &str) -> Result<Ca, CaError> {
        if let Some((cert, key)) = store.load()? {
            return Self::from_pem(&cert, &key);
        }
        let ca = Self::generate(subject_suffix)?;
        store.save(&ca.inner.cert_pem, &ca.inner.key_pem)?;
        Ok(ca)
    }

    /// The CA certificate as PEM (public).
    pub fn cert_pem(&self) -> String {
        self.inner.cert_pem.clone()
    }

    /// The CA certificate as DER (public).
    pub fn cert_der(&self) -> Vec<u8> {
        self.inner.cert_der.clone()
    }

    /// The CA **private key** as PKCS#8 PEM.
    ///
    /// Sensitive: only the key store should call this. It must never reach an
    /// API response, export, log or sync.
    pub fn key_pem(&self) -> String {
        self.inner.key_pem.clone()
    }

    /// SHA-256 of the DER `SubjectPublicKeyInfo`, standard base64, for Chromium's
    /// `--ignore-certificate-errors-spki-list`.
    pub fn spki_sha256_base64(&self) -> String {
        self.inner.spki_pin.clone()
    }
}

fn pem_decode(pem: &str) -> Result<Vec<u8>, CaError> {
    let mut body = String::new();
    let mut inside = false;
    for line in pem.lines() {
        let line = line.trim();
        if line.starts_with("-----BEGIN CERTIFICATE") {
            inside = true;
        } else if line.starts_with("-----END CERTIFICATE") {
            return unbase64(&body).ok_or_else(|| CaError::Pem("bad base64".into()));
        } else if inside {
            body.push_str(line);
        }
    }
    Err(CaError::Pem("no CERTIFICATE block".into()))
}

const ALPHA: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub(crate) fn base64(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= c.len() {
                out.push(ALPHA[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn unbase64(s: &str) -> Option<Vec<u8>> {
    let s = s.trim_end_matches('=');
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0);
    for ch in s.bytes() {
        let v = ALPHA.iter().position(|&a| a == ch)? as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_roundtrip_and_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(unbase64("Zm9vYg==").unwrap(), b"foob");
    }
}
