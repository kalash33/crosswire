//! Body capture: the raw framed bytes of one body as the relay forwards
//! them, kept up to the capture cap while the in-flight budget allows,
//! with the length and SHA-256 of everything that passed.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use bytes::{Bytes, BytesMut};
use crosswire_model::{Body, Capture};
use sha2::{Digest, Sha256};

/// Kept bytes are reserved from the [`Budget`] in steps of this size.
pub(crate) const RESERVE_STEP: u64 = 64 * 1024;

/// The in-flight body budget: the most captured body bytes held across
/// every connection of one listener at once (`body_budget`). Request and
/// response captures draw from the same budget.
#[derive(Debug)]
pub(crate) struct Budget {
    limit: u64,
    used: AtomicU64,
}

impl Budget {
    /// A budget of `limit` bytes, none in use.
    pub(crate) fn new(limit: u64) -> Budget {
        Budget {
            limit,
            used: AtomicU64::new(0),
        }
    }

    /// Takes `n` bytes of the budget, or `None` when that would exceed the
    /// limit. The bytes are given back when the [`Reservation`] drops.
    pub(crate) fn try_reserve(self: &Arc<Self>, n: u64) -> Option<Reservation> {
        self.used
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(n).filter(|&total| total <= self.limit)
            })
            .ok()?;
        Some(Reservation {
            budget: Arc::clone(self),
            n,
        })
    }

    /// Bytes currently reserved.
    #[cfg(test)]
    pub(crate) fn used(&self) -> u64 {
        self.used.load(Ordering::Acquire)
    }
}

/// Bytes held from a [`Budget`]; released on drop.
#[derive(Debug)]
pub(crate) struct Reservation {
    budget: Arc<Budget>,
    n: u64,
}

impl Reservation {
    /// Bytes held.
    fn len(&self) -> u64 {
        self.n
    }

    /// Takes over `other`'s bytes, so one release covers both.
    fn absorb(&mut self, mut other: Reservation) {
        self.n += std::mem::take(&mut other.n);
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if self.n > 0 {
            self.budget.used.fetch_sub(self.n, Ordering::AcqRel);
        }
    }
}

/// Collects one body as the relay forwards it: every byte is counted and
/// hashed; bytes are kept while fewer than `cap` are kept and the budget
/// grants them. A refusal drops what was kept (and its reservation) for
/// the rest of the body.
pub(crate) struct CaptureBuf {
    kept: BytesMut,
    cap: u64,
    total: u64,
    hasher: Sha256,
    dropped: bool,
    reservation: Option<Reservation>,
    budget: Arc<Budget>,
}

/// A finished capture. Holds its budget reservation until dropped: the
/// recorder drops it once the sink has taken the exchange.
#[derive(Debug)]
pub(crate) struct CaptureResult {
    /// The kept prefix (empty when dropped).
    pub(crate) bytes: Bytes,
    /// `Some(cap)` when the body was longer than the cap.
    pub(crate) truncated_at: Option<u64>,
    /// The budget refused the bytes; only `relay` describes the body.
    pub(crate) capture_dropped: bool,
    /// Length and SHA-256 of every byte relayed.
    pub(crate) relay: Capture,
    /// The kept bytes' share of the in-flight budget.
    pub(crate) reservation: Option<Reservation>,
}

impl CaptureBuf {
    /// An empty capture keeping at most `cap` bytes, drawing from `budget`.
    pub(crate) fn new(cap: u64, budget: &Arc<Budget>) -> CaptureBuf {
        CaptureBuf {
            kept: BytesMut::new(),
            cap,
            total: 0,
            hasher: Sha256::new(),
            dropped: false,
            reservation: None,
            budget: Arc::clone(budget),
        }
    }

    /// Records bytes that were just forwarded.
    pub(crate) fn push(&mut self, bytes: &[u8]) {
        self.total += bytes.len() as u64;
        self.hasher.update(bytes);
        let kept = self.kept.len() as u64;
        if self.dropped || kept >= self.cap {
            return;
        }
        let take = (bytes.len() as u64).min(self.cap - kept);
        let need = kept + take;
        let mut reserved = self.reservation.as_ref().map_or(0, Reservation::len);
        while reserved < need {
            let step = RESERVE_STEP.min(self.cap - reserved);
            let Some(r) = self.budget.try_reserve(step) else {
                self.dropped = true;
                self.kept = BytesMut::new();
                self.reservation = None;
                return;
            };
            reserved += step;
            match self.reservation.as_mut() {
                Some(held) => held.absorb(r),
                None => self.reservation = Some(r),
            }
        }
        #[allow(clippy::cast_possible_truncation, reason = "take <= bytes.len()")]
        self.kept.extend_from_slice(&bytes[..take as usize]);
    }

    /// Ends the capture.
    pub(crate) fn finish(self) -> CaptureResult {
        let truncated_at = (!self.dropped && self.total > self.cap).then_some(self.cap);
        CaptureResult {
            bytes: self.kept.freeze(),
            truncated_at,
            capture_dropped: self.dropped,
            relay: Capture {
                len: self.total,
                sha256: self.hasher.finalize().into(),
            },
            reservation: self.reservation,
        }
    }
}

impl CaptureResult {
    /// The body to record: `Body::None` for an empty body, otherwise the
    /// kept bytes inline, with the relay record when they are not the
    /// whole body (cut at the cap, or dropped for the budget).
    pub(crate) fn body(&self) -> Body {
        if self.relay.len == 0 && !self.capture_dropped {
            return Body::None;
        }
        let partial = self.truncated_at.is_some() || self.capture_dropped;
        Body::Inline {
            bytes: self.bytes.clone(),
            truncated_at: self.truncated_at,
            capture_dropped: self.capture_dropped,
            relay: partial.then_some(self.relay),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    const KIB: u64 = 1024;

    fn sha(b: &[u8]) -> [u8; 32] {
        Sha256::digest(b).into()
    }

    #[test]
    fn keeps_prefix_counts_all_hashes_all() {
        let budget = Arc::new(Budget::new(256 * KIB * KIB));
        let mut c = CaptureBuf::new(KIB, &budget);
        let data: Vec<u8> = (0..3 * KIB).map(|i| (i % 251) as u8).collect();
        for piece in data.chunks(700) {
            c.push(piece);
        }
        let r = c.finish();
        assert_eq!(r.bytes.len(), 1024);
        assert_eq!(&r.bytes[..], &data[..1024]);
        assert_eq!(r.truncated_at, Some(1024));
        assert!(!r.capture_dropped);
        assert_eq!(r.relay.len, 3072);
        assert_eq!(r.relay.sha256, sha(&data));
    }

    #[test]
    fn whole_body_under_the_cap_is_not_truncated() {
        let budget = Arc::new(Budget::new(KIB * KIB));
        let mut c = CaptureBuf::new(KIB, &budget);
        c.push(b"hel");
        c.push(b"lo");
        let r = c.finish();
        assert_eq!(&r.bytes[..], b"hello");
        assert_eq!(r.truncated_at, None);
        assert!(!r.capture_dropped);
        assert_eq!(r.relay.len, 5);
        assert_eq!(r.relay.sha256, sha(b"hello"));
    }

    #[test]
    fn budget_refusal_marks_dropped() {
        let budget = Arc::new(Budget::new(KIB));
        let mut c = CaptureBuf::new(50 * KIB * KIB, &budget);
        let data = vec![7u8; 2 * KIB as usize];
        c.push(&data[..KIB as usize]);
        c.push(&data[KIB as usize..]);
        let r = c.finish();
        assert!(r.capture_dropped);
        assert!(r.bytes.is_empty());
        assert_eq!(r.truncated_at, None);
        assert_eq!(r.relay.len, 2048);
        assert_eq!(r.relay.sha256, sha(&data));
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn budget_refusal_mid_body_releases_what_was_kept() {
        // One 64 KiB step fits; the second does not.
        let budget = Arc::new(Budget::new(RESERVE_STEP + KIB));
        let mut c = CaptureBuf::new(KIB * KIB, &budget);
        c.push(&vec![1u8; RESERVE_STEP as usize]);
        assert_eq!(budget.used(), RESERVE_STEP);
        c.push(b"x");
        assert_eq!(budget.used(), 0);
        let r = c.finish();
        assert!(r.capture_dropped);
        assert!(r.bytes.is_empty());
        assert_eq!(r.relay.len, RESERVE_STEP + 1);
    }

    #[test]
    fn reservation_released_on_finish() {
        let budget = Arc::new(Budget::new(KIB * KIB));
        let mut c = CaptureBuf::new(KIB * KIB, &budget);
        c.push(&[0u8; 100]);
        assert_eq!(budget.used(), RESERVE_STEP);
        let r = c.finish();
        // The reservation travels with the result until the recorder has
        // handed the bytes to the sink.
        assert_eq!(budget.used(), RESERVE_STEP);
        drop(r);
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn reservations_are_bounded_by_the_cap() {
        let budget = Arc::new(Budget::new(KIB * KIB));
        let mut c = CaptureBuf::new(KIB, &budget);
        c.push(&[0u8; 4096]);
        assert_eq!(budget.used(), KIB);
        drop(c);
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn two_captures_share_one_budget() {
        let budget = Arc::new(Budget::new(RESERVE_STEP));
        let mut a = CaptureBuf::new(KIB * KIB, &budget);
        let mut b = CaptureBuf::new(KIB * KIB, &budget);
        a.push(b"a");
        b.push(b"b");
        let (a, b) = (a.finish(), b.finish());
        assert!(!a.capture_dropped);
        assert!(b.capture_dropped);
        assert_eq!(b.relay.len, 1);
    }
}
