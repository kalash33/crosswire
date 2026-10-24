//! Bootstrap tokens and API-token hashing.
//!
//! The bootstrap token is a single-use, short-lived, 256-bit secret accepted
//! only by `POST /api/v1/session`. API tokens are compared by looking up the
//! SHA-256 of their 32 raw bytes in the store, so the plaintext secret never
//! enters a non-constant-time comparison.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

/// The prefix every API-token secret carries.
const TOKEN_PREFIX: &str = "cwt_";

/// A minted bootstrap token and its expiry.
#[derive(Clone, Debug)]
struct Bootstrap {
    token: String,
    expires_at: Instant,
}

/// Holds the current single-use bootstrap token, if one is live.
#[derive(Clone, Debug, Default)]
pub struct BootstrapStore {
    inner: Arc<Mutex<Option<Bootstrap>>>,
}

impl BootstrapStore {
    /// A fresh, empty store.
    #[must_use]
    pub fn new() -> BootstrapStore {
        BootstrapStore::default()
    }

    /// Mints a new 256-bit bootstrap token, stores it with `ttl` expiry
    /// (replacing any previous one), and returns the secret.
    #[must_use]
    pub fn mint(&self, ttl: Duration) -> String {
        let raw: [u8; 32] = rand::random();
        let token = URL_SAFE_NO_PAD.encode(raw);
        *self.inner.lock().unwrap_or_else(PoisonError::into_inner) = Some(Bootstrap {
            token: token.clone(),
            expires_at: Instant::now() + ttl,
        });
        token
    }

    /// Consumes the bootstrap token: returns `true` when `presented` matches
    /// the live, unexpired token (constant-time), and removes it so it can
    /// never be used again. A wrong, expired, or already-used token returns
    /// `false` and leaves any live token in place.
    #[must_use]
    pub fn consume(&self, presented: &str) -> bool {
        let mut slot = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(current) = slot.as_ref() else {
            return false;
        };
        if Instant::now() >= current.expires_at {
            *slot = None;
            return false;
        }
        if bool::from(current.token.as_bytes().ct_eq(presented.as_bytes())) {
            *slot = None;
            true
        } else {
            false
        }
    }
}

/// Decodes an `Authorization: Bearer cwt_<base64url>` value into the SHA-256
/// hash of the token's 32 raw bytes, ready for [`Control::token_lookup`].
///
/// Returns `None` when the value is not a well-formed token (wrong prefix,
/// bad base64url, or not exactly 32 bytes).
///
/// [`Control::token_lookup`]: crosswire_store::Control::token_lookup
#[must_use]
pub fn token_hash(bearer: &str) -> Option<[u8; 32]> {
    let b64 = bearer.strip_prefix(TOKEN_PREFIX)?;
    let raw = URL_SAFE_NO_PAD.decode(b64).ok()?;
    let raw: [u8; 32] = raw.try_into().ok()?;
    Some(Sha256::digest(raw).into())
}
