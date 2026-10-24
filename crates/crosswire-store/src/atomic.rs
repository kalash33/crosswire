//! Compare-and-update helpers for the counters shared between threads.
//!
//! Rust 1.99 renamed `fetch_update` to `try_update` and deprecated the old
//! name. The workspace's minimum supported Rust is 1.88, which only has the
//! old name, so the deprecated call is confined to this module and every
//! other site goes through these wrappers.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// Applies `f` to the current value until it succeeds or `f` returns `None`.
///
/// Returns the previous value on success and the current value when `f`
/// declined, exactly like `AtomicU64::try_update` with `SeqCst` ordering.
#[allow(deprecated)]
pub(crate) fn try_update_u64(
    atomic: &AtomicU64,
    f: impl FnMut(u64) -> Option<u64>,
) -> Result<u64, u64> {
    atomic.fetch_update(Ordering::SeqCst, Ordering::SeqCst, f)
}

/// [`try_update_u64`] for `usize` counters.
#[allow(deprecated)]
pub(crate) fn try_update_usize(
    atomic: &AtomicUsize,
    f: impl FnMut(usize) -> Option<usize>,
) -> Result<usize, usize> {
    atomic.fetch_update(Ordering::SeqCst, Ordering::SeqCst, f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn u64_updates_until_declined() {
        let a = AtomicU64::new(5);
        assert_eq!(try_update_u64(&a, |v| v.checked_add(1)), Ok(5));
        assert_eq!(a.load(Ordering::SeqCst), 6);
        assert_eq!(try_update_u64(&a, |_| None), Err(6));
        assert_eq!(a.load(Ordering::SeqCst), 6);
    }

    #[test]
    fn usize_updates_until_declined() {
        let a = AtomicUsize::new(1);
        assert_eq!(try_update_usize(&a, |v| v.checked_sub(1)), Ok(1));
        assert_eq!(try_update_usize(&a, |v| v.checked_sub(1)), Err(0));
    }
}
