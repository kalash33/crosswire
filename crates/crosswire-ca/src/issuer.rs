//! Cached, rate-limited per-host leaf issuance.

use std::net::IpAddr;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use lru::LruCache;
use rcgen::{
    CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, KeyPair,
    KeyUsagePurpose, SanType, PKCS_ECDSA_P256_SHA256,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName};
use rustls::sign::CertifiedKey;
use time::{Duration, OffsetDateTime};

use crate::ca::{random_serial, Ca};
use crate::error::CaError;

/// Leaf validity cap (browser maximum).
const LEAF_VALIDITY_DAYS: i64 = 398;

/// Limits for a [`LeafIssuer`].
#[derive(Debug, Clone, Copy)]
pub struct IssuerLimits {
    /// Maximum cached leaves (LRU eviction beyond this). Default 4096.
    pub cache_entries: usize,
    /// Leaves that may be minted per second (token bucket; cache hits are free). Default 50.
    pub mint_rate_per_sec: u32,
}

impl Default for IssuerLimits {
    fn default() -> Self {
        IssuerLimits {
            cache_entries: 4096,
            mint_rate_per_sec: 50,
        }
    }
}

struct State {
    cache: LruCache<String, Arc<CertifiedKey>>,
    tokens: f64,
    last: Instant,
}

/// Mints and caches leaf certificates signed by a [`Ca`].
///
/// One P-256 leaf key pair is generated per issuer and reused for every host.
pub struct LeafIssuer {
    ca: Arc<Ca>,
    leaf_key: KeyPair,
    leaf_pkcs8: Vec<u8>,
    rate: f64,
    state: Mutex<State>,
}

impl std::fmt::Debug for LeafIssuer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LeafIssuer").finish_non_exhaustive()
    }
}

impl LeafIssuer {
    /// Create an issuer for `ca`.
    ///
    /// # Panics
    /// If the system random source cannot generate a key pair.
    pub fn new(ca: Arc<Ca>, limits: IssuerLimits) -> LeafIssuer {
        let leaf_key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)
            .expect("P-256 key generation failed: no system randomness");
        let leaf_pkcs8 = leaf_key.serialize_der();
        let cap = NonZeroUsize::new(limits.cache_entries.max(1)).expect("max(1) is non-zero");
        let rate = f64::from(limits.mint_rate_per_sec.max(1));
        LeafIssuer {
            ca,
            leaf_key,
            leaf_pkcs8,
            rate,
            state: Mutex::new(State {
                cache: LruCache::new(cap),
                tokens: rate,
                last: Instant::now(),
            }),
        }
    }

    /// Number of cached leaves.
    pub fn cached_len(&self) -> usize {
        self.lock().cache.len()
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Issue (or fetch from cache) a leaf for `host`: a DNS SAN for names, an
    /// IP SAN for literals. Returns [`CaError::RateLimited`] when a new leaf is
    /// needed and the mint rate is exhausted.
    pub fn issue(&self, host: &ServerName<'_>) -> Result<Arc<CertifiedKey>, CaError> {
        let (key, san) = match host {
            ServerName::DnsName(d) => {
                let name = d.as_ref().to_ascii_lowercase();
                let ia5 = name.clone().try_into().map_err(|_| CaError::InvalidHost)?;
                (name, SanType::DnsName(ia5))
            }
            ServerName::IpAddress(ip) => {
                let ip = IpAddr::from(*ip);
                (ip.to_string(), SanType::IpAddress(ip))
            }
            _ => return Err(CaError::InvalidHost),
        };

        let mut st = self.lock();
        if let Some(hit) = st.cache.get(&key) {
            return Ok(Arc::clone(hit));
        }
        let now = Instant::now();
        let elapsed = now.duration_since(st.last).as_secs_f64();
        st.tokens = (st.tokens + elapsed * self.rate).min(self.rate);
        st.last = now;
        if st.tokens < 1.0 {
            return Err(CaError::RateLimited);
        }
        let ck = Arc::new(self.mint(&key, san)?);
        st.tokens -= 1.0;
        st.cache.put(key, Arc::clone(&ck));
        Ok(ck)
    }

    fn mint(&self, cn: &str, san: SanType) -> Result<CertifiedKey, CaError> {
        let gen = |e: &dyn std::fmt::Display| CaError::Generate(e.to_string());
        let now = OffsetDateTime::now_utc();
        let mut params = CertificateParams::default();
        params.not_before = now - Duration::days(1);
        params.not_after = (now + Duration::days(LEAF_VALIDITY_DAYS))
            .min(self.ca.inner.not_after - Duration::days(1));
        params.serial_number = Some(random_serial()?);
        params.distinguished_name = DistinguishedName::new();
        params.distinguished_name.push(DnType::CommonName, cn);
        params.subject_alt_names = vec![san];
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        params.use_authority_key_identifier_extension = true;
        let cert = params
            .signed_by(&self.leaf_key, &self.ca.inner.issuer)
            .map_err(|e| gen(&e))?;
        let signing_key = rustls::crypto::ring::sign::any_ecdsa_type(&PrivateKeyDer::Pkcs8(
            PrivatePkcs8KeyDer::from(self.leaf_pkcs8.clone()),
        ))
        .map_err(|e| gen(&e))?;
        Ok(CertifiedKey::new(
            vec![
                CertificateDer::from(cert.der().to_vec()),
                CertificateDer::from(self.ca.cert_der()),
            ],
            signing_key,
        ))
    }
}
