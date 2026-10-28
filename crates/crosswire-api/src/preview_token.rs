//! Per-exchange HMAC capability token for the isolated preview origin (§11.1).
//!
//! The preview layer serves captured HTML and image bodies for viewing on a
//! different browser origin, so a captured page can never read the API's
//! cookies or storage. That origin carries no cookie: the capability token
//! *is* the access credential. Each token is bound to one exchange id and to
//! the exact bytes of its body, signed with a per-serve secret, and expires
//! after a short time-to-live.
//!
//! The token is
//! `base64url(expiry_be_u64 || HMAC_SHA256(key, ulid || body_hash || expiry_be_u64))`.
//! [`verify`] recomputes the MAC over the same preimage, compares it in
//! constant time, and rejects anything whose expiry is not in the future. A
//! token minted for a different exchange, a changed body, or a flipped byte
//! recomputes to a different MAC and is refused.
//!
//! HMAC-SHA256 is built here from `sha2` (the standard ipad/opad
//! construction), so the preview capability pulls in no new dependency.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use ulid::Ulid;

/// The SHA-256 block size, in bytes.
const BLOCK: usize = 64;
/// The HMAC-SHA256 output size, in bytes.
const MAC_LEN: usize = 32;
/// The big-endian expiry prefix size, in bytes.
const EXP_LEN: usize = 8;

/// A per-serve secret for minting and verifying preview capability tokens.
///
/// Generated once at serve start and held on the app state. It is never
/// serialized, logged, or exposed over the API; [`Debug`] hides its bytes.
pub struct PreviewKey([u8; 32]);

impl PreviewKey {
    /// A fresh 256-bit key from the OS RNG.
    #[must_use]
    pub fn random() -> PreviewKey {
        PreviewKey(rand::random())
    }
}

impl std::fmt::Debug for PreviewKey {
    /// Never renders the secret bytes.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreviewKey").finish_non_exhaustive()
    }
}

/// Why a preview token was rejected.
///
/// The variant is deliberately coarse: a caller must not be able to learn
/// *why* a token failed (forged MAC, wrong exchange, wrong body, or expired),
/// because distinguishing those leaks information about the secret.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PreviewTokenError {
    /// The token is malformed, forged, for a different exchange or body, or
    /// expired. The reason is not distinguished on purpose.
    #[error("invalid preview token")]
    Invalid,
}

/// HMAC-SHA256 over `msg` with `key`, the standard ipad/opad construction
/// (RFC 2104). Keys longer than the block are hashed first; shorter keys are
/// zero-padded to one block.
fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; MAC_LEN] {
    let mut block = [0u8; BLOCK];
    if key.len() > BLOCK {
        block[..MAC_LEN].copy_from_slice(&Sha256::digest(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }

    let mut ipad = [0x36u8; BLOCK];
    let mut opad = [0x5cu8; BLOCK];
    for i in 0..BLOCK {
        ipad[i] ^= block[i];
        opad[i] ^= block[i];
    }

    let mut inner = Sha256::new();
    inner.update(ipad);
    inner.update(msg);
    let inner = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(inner);
    outer.finalize().into()
}

/// The signed preimage: `ulid || body_hash || expiry_be`.
fn preimage(ulid: Ulid, body_hash: &[u8; 32], expiry_be: &[u8; EXP_LEN]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(16 + 32 + EXP_LEN);
    buf.extend_from_slice(&ulid.to_bytes());
    buf.extend_from_slice(body_hash);
    buf.extend_from_slice(expiry_be);
    buf
}

/// The absolute expiry, in whole unix seconds, of a token minted at `now`
/// with lifetime `ttl`. Saturating, so a far-future `ttl` cannot wrap.
fn expiry_secs(now: SystemTime, ttl: Duration) -> u64 {
    now.duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .saturating_add(ttl)
        .as_secs()
}

/// Mints a capability token binding `ulid` and `body_hash`, valid for `ttl`
/// from `now`.
///
/// The token is
/// `base64url(expiry_be_u64 || HMAC_SHA256(key, ulid || body_hash || expiry_be_u64))`,
/// with no padding.
#[must_use]
pub fn mint(
    key: &PreviewKey,
    ulid: Ulid,
    body_hash: &[u8; 32],
    now: SystemTime,
    ttl: Duration,
) -> String {
    let expiry_be = expiry_secs(now, ttl).to_be_bytes();
    let mac = hmac_sha256(&key.0, &preimage(ulid, body_hash, &expiry_be));
    let mut raw = Vec::with_capacity(EXP_LEN + MAC_LEN);
    raw.extend_from_slice(&expiry_be);
    raw.extend_from_slice(&mac);
    URL_SAFE_NO_PAD.encode(raw)
}

/// Verifies a capability `token` against `ulid`, `body_hash`, and the current
/// time `now`.
///
/// Decodes the token, recomputes the HMAC over the same preimage, compares the
/// full MAC in constant time, and requires the expiry to be strictly in the
/// future. Any mismatch, malformed token, or expiry at or before `now` returns
/// [`PreviewTokenError::Invalid`] without revealing which.
///
/// # Errors
/// [`PreviewTokenError::Invalid`] for a forged, malformed, mismatched, or
/// expired token.
pub fn verify(
    key: &PreviewKey,
    ulid: Ulid,
    body_hash: &[u8; 32],
    token: &str,
    now: SystemTime,
) -> Result<(), PreviewTokenError> {
    let raw = URL_SAFE_NO_PAD
        .decode(token)
        .map_err(|_| PreviewTokenError::Invalid)?;
    if raw.len() != EXP_LEN + MAC_LEN {
        return Err(PreviewTokenError::Invalid);
    }
    let mut expiry_be = [0u8; EXP_LEN];
    expiry_be.copy_from_slice(&raw[..EXP_LEN]);
    let presented_mac = &raw[EXP_LEN..];

    let expected = hmac_sha256(&key.0, &preimage(ulid, body_hash, &expiry_be));
    // Constant-time compare the full MAC before the expiry branch.
    if !bool::from(expected.as_slice().ct_eq(presented_mac)) {
        return Err(PreviewTokenError::Invalid);
    }

    let expiry = u64::from_be_bytes(expiry_be);
    let now_secs = now
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs();
    if expiry > now_secs {
        Ok(())
    } else {
        Err(PreviewTokenError::Invalid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed_key() -> PreviewKey {
        PreviewKey([7u8; 32])
    }

    fn body_hash() -> [u8; 32] {
        [0x42u8; 32]
    }

    #[test]
    fn mint_then_verify_ok() {
        let key = fixed_key();
        let ulid = Ulid::new();
        let now = SystemTime::now();
        let token = mint(&key, ulid, &body_hash(), now, Duration::from_secs(60));
        assert_eq!(verify(&key, ulid, &body_hash(), &token, now), Ok(()));
    }

    #[test]
    fn verify_rejects_wrong_exchange() {
        let key = fixed_key();
        let a = Ulid::new();
        let b = Ulid::new();
        assert_ne!(a, b);
        let now = SystemTime::now();
        let token = mint(&key, a, &body_hash(), now, Duration::from_secs(60));
        assert_eq!(
            verify(&key, b, &body_hash(), &token, now),
            Err(PreviewTokenError::Invalid)
        );
    }

    #[test]
    fn verify_rejects_changed_body_hash() {
        let key = fixed_key();
        let ulid = Ulid::new();
        let now = SystemTime::now();
        let token = mint(&key, ulid, &body_hash(), now, Duration::from_secs(60));
        let mut other = body_hash();
        other[0] ^= 0x01;
        assert_eq!(
            verify(&key, ulid, &other, &token, now),
            Err(PreviewTokenError::Invalid)
        );
    }

    #[test]
    fn verify_rejects_expired() {
        let key = fixed_key();
        let ulid = Ulid::new();
        let minted_at = UNIX_EPOCH + Duration::from_secs(1_000);
        let token = mint(&key, ulid, &body_hash(), minted_at, Duration::from_secs(60));
        // The token expired at 1_060; check well after that.
        let later = UNIX_EPOCH + Duration::from_secs(2_000);
        assert_eq!(
            verify(&key, ulid, &body_hash(), &token, later),
            Err(PreviewTokenError::Invalid)
        );
        // It was valid before expiry.
        assert_eq!(verify(&key, ulid, &body_hash(), &token, minted_at), Ok(()));
    }

    #[test]
    fn verify_rejects_tampered() {
        let key = fixed_key();
        let ulid = Ulid::new();
        let now = SystemTime::now();
        let token = mint(&key, ulid, &body_hash(), now, Duration::from_secs(60));
        // Flip a byte in the base64url payload (the final MAC region).
        let mut bytes = token.into_bytes();
        let last = bytes.len() - 1;
        bytes[last] = if bytes[last] == b'A' { b'B' } else { b'A' };
        let tampered = String::from_utf8(bytes).unwrap();
        assert_eq!(
            verify(&key, ulid, &body_hash(), &tampered, now),
            Err(PreviewTokenError::Invalid)
        );
    }

    #[test]
    fn wrong_key_rejected() {
        let ulid = Ulid::new();
        let now = SystemTime::now();
        let token = mint(
            &fixed_key(),
            ulid,
            &body_hash(),
            now,
            Duration::from_secs(60),
        );
        let other = PreviewKey([8u8; 32]);
        assert_eq!(
            verify(&other, ulid, &body_hash(), &token, now),
            Err(PreviewTokenError::Invalid)
        );
    }

    /// RFC 4231, Test Case 1: key = 0x0b * 20, data = "Hi There".
    #[test]
    fn hmac_sha256_rfc4231_vector() {
        let key = [0x0bu8; 20];
        let data = b"Hi There";
        let expected = hex("b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7");
        assert_eq!(hmac_sha256(&key, data).as_slice(), expected.as_slice());
    }

    /// RFC 4231, Test Case 2: key = "Jefe", data = "what do ya want for ...".
    #[test]
    fn hmac_sha256_rfc4231_vector_2() {
        let key = b"Jefe";
        let data = b"what do ya want for nothing?";
        let expected = hex("5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
        assert_eq!(hmac_sha256(key, data).as_slice(), expected.as_slice());
    }

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }
}
