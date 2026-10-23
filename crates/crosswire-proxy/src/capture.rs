//! Body capture: the raw framed bytes of one body as the relay forwards
//! them, kept up to the capture cap while the in-flight budget allows,
//! with the length and SHA-256 of everything that passed.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use bytes::Bytes;
use crosswire_model::{Body, Capture};
use sha2::{Digest, Sha256};

/// The first capacity of a capture buffer; each growth doubles it, up to
/// the cap.
pub(crate) const FIRST_STEP: u64 = 4 * 1024;

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
    /// Grown only by [`CaptureBuf::grow`], so its capacity never exceeds
    /// the reservation.
    kept: Vec<u8>,
    cap: u64,
    total: u64,
    hasher: Sha256,
    dropped: bool,
    reservation: Option<Reservation>,
    budget: Arc<Budget>,
    /// Times the buffer grew (tests only).
    #[cfg(test)]
    growths: u32,
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
            kept: Vec::new(),
            cap,
            total: 0,
            hasher: Sha256::new(),
            dropped: false,
            reservation: None,
            budget: Arc::clone(budget),
            #[cfg(test)]
            growths: 0,
        }
    }

    fn reserved(&self) -> u64 {
        self.reservation.as_ref().map_or(0, Reservation::len)
    }

    /// Adds `n` bytes to the reservation; `false` (and nothing taken) when
    /// the budget refuses.
    fn reserve(&mut self, n: u64) -> bool {
        let Some(r) = self.budget.try_reserve(n) else {
            return false;
        };
        match self.reservation.as_mut() {
            Some(held) => held.absorb(r),
            None => self.reservation = Some(r),
        }
        true
    }

    /// Doubles the buffer's capacity (at least [`FIRST_STEP`], never past
    /// `cap`), reserving the whole increase from the budget first. The
    /// reservation always covers the capacity, so capture memory stays
    /// within the budget; the copy during a growth briefly also holds the
    /// old buffer, which is not charged.
    fn grow(&mut self) -> bool {
        #[cfg(test)]
        {
            self.growths += 1;
        }
        let capacity = self.kept.capacity() as u64;
        let target = capacity.saturating_mul(2).max(FIRST_STEP).min(self.cap);
        let step = target.saturating_sub(capacity);
        if step == 0 || !self.reserve(step) {
            return false;
        }
        #[allow(
            clippy::cast_possible_truncation,
            reason = "bounded by cap, which fits in memory"
        )]
        self.kept
            .reserve_exact((capacity + step) as usize - self.kept.len());
        // The allocator may round up; charge any excess too.
        let excess = (self.kept.capacity() as u64).saturating_sub(self.reserved());
        if excess > 0 && !self.reserve(excess) {
            return false;
        }
        debug_assert!(self.kept.capacity() as u64 <= self.reserved());
        true
    }

    /// Frees the buffer and its reservation; the rest of the body is only
    /// counted and hashed.
    fn drop_capture(&mut self) {
        self.dropped = true;
        self.kept = Vec::new();
        self.reservation = None;
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
        while (self.kept.capacity() as u64) < kept + take {
            if !self.grow() {
                self.drop_capture();
                return;
            }
        }
        #[allow(clippy::cast_possible_truncation, reason = "take <= bytes.len()")]
        self.kept.extend_from_slice(&bytes[..take as usize]);
        debug_assert!(self.kept.capacity() as u64 <= self.reserved());
    }

    /// Ends the capture.
    pub(crate) fn finish(self) -> CaptureResult {
        let truncated_at = (!self.dropped && self.total > self.cap).then_some(self.cap);
        CaptureResult {
            bytes: Bytes::from(self.kept),
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
        // Growth to 16 KiB fits; the next growth (to 32 KiB) does not.
        let budget = Arc::new(Budget::new(16 * KIB));
        let mut c = CaptureBuf::new(KIB * KIB, &budget);
        c.push(&vec![1u8; 16 * KIB as usize]);
        assert_eq!(budget.used(), 16 * KIB);
        c.push(b"x");
        assert_eq!(budget.used(), 0);
        let r = c.finish();
        assert!(r.capture_dropped);
        assert!(r.bytes.is_empty());
        assert_eq!(r.relay.len, 16 * KIB + 1);
    }

    #[test]
    fn growth_is_geometric() {
        let budget = Arc::new(Budget::new(64 * KIB * KIB));
        let cap = 50 * KIB * KIB;
        let slice = vec![3u8; 64 * KIB as usize];
        // Uncapped: 3 MiB + 1 byte ends in a 4 MiB buffer.
        let mut c = CaptureBuf::new(cap, &budget);
        for _ in 0..48 {
            c.push(&slice);
        }
        c.push(b"+");
        let kept = c.kept.len() as u64;
        assert_eq!(kept, 3 * KIB * KIB + 1);
        assert!(c.kept.capacity() as u64 <= 2 * kept);
        assert_eq!(c.reserved(), c.kept.capacity() as u64);
        drop(c);
        // Capped: 50 MiB in 64 KiB slices. 4 KiB doubling to 32 MiB is 14
        // growths, plus one to the cap.
        let mut c = CaptureBuf::new(cap, &budget);
        for _ in 0..cap / (64 * KIB) {
            c.push(&slice);
        }
        assert!(c.growths <= 15, "{} growths", c.growths);
        assert_eq!(c.kept.len() as u64, cap);
        assert_eq!(c.kept.capacity() as u64, cap);
        assert_eq!(budget.used(), cap);
    }

    #[test]
    fn reservation_released_on_finish() {
        let budget = Arc::new(Budget::new(KIB * KIB));
        let mut c = CaptureBuf::new(KIB * KIB, &budget);
        c.push(&[0u8; 100]);
        assert_eq!(budget.used(), FIRST_STEP);
        let r = c.finish();
        // The reservation travels with the result until the recorder has
        // handed the bytes to the sink.
        assert_eq!(budget.used(), FIRST_STEP);
        drop(r);
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn small_bodies_reserve_small() {
        let budget = Arc::new(Budget::new(KIB * KIB));
        let mut c = CaptureBuf::new(50 * KIB * KIB, &budget);
        c.push(&[9u8; 100]);
        assert_eq!(budget.used(), 4 * KIB);
        assert_eq!(c.finish().bytes.len(), 100);
    }

    #[test]
    fn capacity_never_exceeds_reservation() {
        let budget = Arc::new(Budget::new(KIB * KIB));
        let mut c = CaptureBuf::new(KIB * KIB, &budget);
        for i in 0..200 * KIB {
            c.push(&[(i % 251) as u8]);
            assert!(c.kept.capacity() as u64 <= c.reserved(), "at byte {i}");
        }
        assert_eq!(c.reserved(), c.kept.capacity() as u64);
        assert_eq!(budget.used(), c.kept.capacity() as u64);
        // Doubling from 4 KiB: the first capacity holding 200 KiB.
        assert_eq!(c.reserved(), 256 * KIB);
        let r = c.finish();
        assert_eq!(r.bytes.len() as u64, 200 * KIB);
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
        let budget = Arc::new(Budget::new(FIRST_STEP));
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
