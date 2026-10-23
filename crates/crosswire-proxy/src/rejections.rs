//! Counting client TLS handshake rejections per host.
//!
//! A browser that does not trust the Crosswire CA aborts every handshake
//! with a leaf from it. The proxy counts those per host and, when
//! `rejections_per_window` land inside `rejection_window`, reports it once
//! per window. It never switches the host to passthrough on its own.

use std::collections::VecDeque;
use std::num::NonZeroUsize;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use lru::LruCache;

/// Most hosts tracked; the least recently rejected is forgotten first.
const MAX_HOSTS: usize = 1_024;

struct HostRejections {
    /// Recent rejection times, oldest first; at most `threshold` kept.
    times: VecDeque<Instant>,
    /// When this host was last reported.
    reported: Option<Instant>,
}

/// Per-host sliding-window counter of rejected handshakes.
pub(crate) struct RejectionTracker {
    hosts: Mutex<LruCache<String, HostRejections>>,
    threshold: usize,
    window: Duration,
}

impl RejectionTracker {
    /// A tracker that reports `threshold` rejections within `window`
    /// (a threshold of 0 is treated as 1).
    pub(crate) fn new(threshold: u32, window: Duration) -> RejectionTracker {
        RejectionTracker {
            hosts: Mutex::new(LruCache::new(
                NonZeroUsize::new(MAX_HOSTS).expect("non-zero constant"),
            )),
            threshold: usize::try_from(threshold.max(1)).unwrap_or(usize::MAX),
            window,
        }
    }

    /// Records one rejection for `host` (case-insensitive) at `now`.
    /// Returns `true` when this makes `threshold` rejections within the
    /// window and the host has not been reported within the last window:
    /// the caller then raises the marker.
    pub(crate) fn record(&self, host: &str, now: Instant) -> bool {
        let key = host.trim_end_matches('.').to_ascii_lowercase();
        let mut hosts = self.hosts.lock().unwrap_or_else(PoisonError::into_inner);
        let entry = hosts.get_or_insert_mut(key, || HostRejections {
            times: VecDeque::with_capacity(self.threshold),
            reported: None,
        });
        while entry
            .times
            .front()
            .is_some_and(|&t| now.saturating_duration_since(t) >= self.window)
        {
            entry.times.pop_front();
        }
        if entry.times.len() == self.threshold {
            entry.times.pop_front();
        }
        entry.times.push_back(now);
        let due = entry
            .reported
            .is_none_or(|r| now.saturating_duration_since(r) >= self.window);
        if entry.times.len() >= self.threshold && due {
            entry.reported = Some(now);
            return true;
        }
        false
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.hosts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_slides() {
        let t = RejectionTracker::new(3, Duration::from_secs(30));
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        // Spread wider than the window: never three at once.
        assert!(!t.record("a.test", at(0)));
        assert!(!t.record("a.test", at(31)));
        assert!(!t.record("a.test", at(62)));
        // Three inside one window: reported on the third.
        assert!(!t.record("b.test", at(0)));
        assert!(!t.record("b.test", at(10)));
        assert!(t.record("b.test", at(20)));
        // Still over the threshold, but already reported in this window.
        assert!(!t.record("b.test", at(25)));
        assert!(!t.record("b.test", at(45)));
        // A window after the report, still three recent: reported again.
        assert!(t.record("b.test", at(51)));
        // Hosts are independent and case-insensitive.
        assert!(!t.record("C.test", at(0)));
        assert!(!t.record("c.TEST", at(1)));
        assert!(t.record("c.test", at(2)));
    }

    #[test]
    fn host_table_is_bounded() {
        let t = RejectionTracker::new(2, Duration::from_secs(30));
        let t0 = Instant::now();
        assert!(!t.record("first.test", t0));
        for i in 0..2_000 {
            let _ = t.record(&format!("h{i}.test"), t0);
        }
        assert!(t.len() <= 1_024);
        // The first host was evicted, so its count starts over.
        assert!(!t.record("first.test", t0));
    }
}
