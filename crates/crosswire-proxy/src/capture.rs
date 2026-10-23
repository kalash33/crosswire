//! Body capture. Placeholder until Task 6 replaces it with the bounded
//! buffer (capture cap, in-flight budget): this one keeps every byte.

use bytes::{Bytes, BytesMut};
use sha2::{Digest, Sha256};

/// Collects the raw framed bytes of one body as the relay forwards them,
/// with their running length and SHA-256.
pub(crate) struct Capture {
    kept: BytesMut,
    len: u64,
    hasher: Sha256,
}

/// A finished capture.
#[derive(Clone, Debug)]
pub(crate) struct CaptureResult {
    /// The captured bytes (all of them, in this placeholder).
    pub(crate) bytes: Bytes,
    /// Bytes relayed.
    pub(crate) len: u64,
    /// SHA-256 of the relayed bytes.
    #[allow(dead_code, reason = "Task 6 records it as the body's relay capture")]
    pub(crate) sha256: [u8; 32],
}

impl Capture {
    /// An empty capture.
    pub(crate) fn new() -> Capture {
        Capture {
            kept: BytesMut::new(),
            len: 0,
            hasher: Sha256::new(),
        }
    }

    /// Appends bytes that were just forwarded.
    pub(crate) fn push(&mut self, bytes: &[u8]) {
        self.kept.extend_from_slice(bytes);
        self.len += bytes.len() as u64;
        self.hasher.update(bytes);
    }

    /// Ends the capture.
    pub(crate) fn finish(self) -> CaptureResult {
        CaptureResult {
            bytes: self.kept.freeze(),
            len: self.len,
            sha256: self.hasher.finalize().into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_counts_and_hashes() {
        let mut c = Capture::new();
        c.push(b"hel");
        c.push(b"lo");
        let r = c.finish();
        assert_eq!(&r.bytes[..], b"hello");
        assert_eq!(r.len, 5);
        let want: [u8; 32] = Sha256::digest(b"hello").into();
        assert_eq!(r.sha256, want);
    }
}
