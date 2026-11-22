//! Per-stage writer timers for the ingest harness (`perf_ingest`).
//!
//! The `stage!` macro wraps one stage of the writer's per-exchange or
//! per-batch work. Without the non-default `stage-timing` feature it expands
//! to the wrapped expression alone, so the release binary carries no timer,
//! no counter and no symbol from here. With the feature it adds the elapsed
//! nanoseconds and one call to a process-wide [`StageTimes`] (relaxed atomics;
//! the writer thread is the only one that records).
//!
//! Only instrumentation: nothing here changes what the writer does or in
//! which order. Wrap a fallible call *without* its `?` (put the `?` after the
//! macro) so the time of a failing call is still recorded.

/// Times `$e` as stage `$stage` (a [`Stage`] variant) and returns its value.
#[cfg(feature = "stage-timing")]
macro_rules! stage {
    ($stage:ident, $e:expr) => {{
        let started = ::std::time::Instant::now();
        let value = $e;
        $crate::stages::STAGES.record($crate::stages::Stage::$stage, started.elapsed());
        value
    }};
}

/// Without `stage-timing`: just `$e`.
#[cfg(not(feature = "stage-timing"))]
macro_rules! stage {
    ($stage:ident, $e:expr) => {
        $e
    };
}

#[cfg(feature = "stage-timing")]
pub use timing::*;

#[cfg(feature = "stage-timing")]
mod timing {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    /// One timed stage of the writer (spec §3.3, stages a–k, plus the batch
    /// totals the stages are a share of).
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Stage {
        /// (a) `SELECT 1 FROM exchanges WHERE ulid = ?`.
        Exists,
        /// (b) Building the row JSON: `to_value(ex)` (0.1.1 base64-encoded the bodies here; 0.1.2 no longer does),
        /// the `state`/`blob` swaps and `to_string`.
        Json,
        /// (d) SHA-256 of a pack body in `BodyStore::put` (the dedup key).
        ShaPut,
        /// (g) SHA-256 of the same pack body again in `PackWriter::append`.
        ShaAppend,
        /// (c) SHA-256 of an inline body for the JSON `hash`.
        ShaInline,
        /// (f) `zstd::bulk::compress` at level 3.
        Zstd,
        /// (h) Building the record (header + frame copy) and `write_all`.
        PackWrite,
        /// (c, e, i) Body SQL: the inline insert, the dedup `SELECT`, the
        /// `refcount` update, `INSERT OR IGNORE INTO packs`, the `blobs` and
        /// `bodies` inserts.
        SqlBodies,
        /// (j) `INSERT INTO exchanges` and its `exchange_tags` rows.
        SqlExchange,
        /// Per batch: `BEGIN IMMEDIATE`, markers, the `meta` updates and
        /// `committed_len`.
        SqlBatch,
        /// (k) The pack `sync_all` (`F_FULLFSYNC` on macOS), once per batch.
        Fsync,
        /// (k) `COMMIT`, once per batch.
        Commit,
        /// The whole `try_commit` attempt (every stage above runs inside it).
        Batch,
        /// After `COMMIT`: queue release, journal-file deletion and events.
        AfterCommit,
    }

    impl Stage {
        /// Every stage, in report order.
        pub const ALL: [Stage; 14] = [
            Stage::Exists,
            Stage::Json,
            Stage::ShaPut,
            Stage::ShaAppend,
            Stage::ShaInline,
            Stage::Zstd,
            Stage::PackWrite,
            Stage::SqlBodies,
            Stage::SqlExchange,
            Stage::SqlBatch,
            Stage::Fsync,
            Stage::Commit,
            Stage::Batch,
            Stage::AfterCommit,
        ];

        /// The stages inside `Batch` (their sum is at most `Batch`).
        pub const INNER: [Stage; 12] = [
            Stage::Exists,
            Stage::Json,
            Stage::ShaPut,
            Stage::ShaAppend,
            Stage::ShaInline,
            Stage::Zstd,
            Stage::PackWrite,
            Stage::SqlBodies,
            Stage::SqlExchange,
            Stage::SqlBatch,
            Stage::Fsync,
            Stage::Commit,
        ];

        /// The snake-case name the harness prints.
        #[must_use]
        pub fn name(self) -> &'static str {
            match self {
                Stage::Exists => "exists",
                Stage::Json => "json",
                Stage::ShaPut => "sha_put",
                Stage::ShaAppend => "sha_append",
                Stage::ShaInline => "sha_inline",
                Stage::Zstd => "zstd",
                Stage::PackWrite => "pack_write",
                Stage::SqlBodies => "sql_bodies",
                Stage::SqlExchange => "sql_exchange",
                Stage::SqlBatch => "sql_batch",
                Stage::Fsync => "fsync",
                Stage::Commit => "commit",
                Stage::Batch => "batch",
                Stage::AfterCommit => "after_commit",
            }
        }

        fn index(self) -> usize {
            self as usize
        }
    }

    /// Accumulated nanoseconds and calls per [`Stage`].
    #[derive(Debug)]
    pub struct StageTimes {
        nanos: [AtomicU64; Stage::ALL.len()],
        calls: [AtomicU64; Stage::ALL.len()],
    }

    impl StageTimes {
        /// All zero.
        #[must_use]
        pub const fn new() -> StageTimes {
            StageTimes {
                nanos: [const { AtomicU64::new(0) }; Stage::ALL.len()],
                calls: [const { AtomicU64::new(0) }; Stage::ALL.len()],
            }
        }

        /// Adds one call of `stage` that took `took`.
        pub fn record(&self, stage: Stage, took: Duration) {
            let ns = u64::try_from(took.as_nanos()).unwrap_or(u64::MAX);
            self.nanos[stage.index()].fetch_add(ns, Ordering::Relaxed);
            self.calls[stage.index()].fetch_add(1, Ordering::Relaxed);
        }

        /// The totals so far.
        #[must_use]
        pub fn snapshot(&self) -> StageSnapshot {
            StageSnapshot {
                nanos: std::array::from_fn(|i| self.nanos[i].load(Ordering::Relaxed)),
                calls: std::array::from_fn(|i| self.calls[i].load(Ordering::Relaxed)),
            }
        }
    }

    impl Default for StageTimes {
        fn default() -> StageTimes {
            StageTimes::new()
        }
    }

    /// A copy of [`StageTimes`] at one instant; subtract two with
    /// [`StageSnapshot::since`] to get one run's share.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct StageSnapshot {
        nanos: [u64; Stage::ALL.len()],
        calls: [u64; Stage::ALL.len()],
    }

    impl StageSnapshot {
        /// Total time spent in `stage`.
        #[must_use]
        pub fn total(&self, stage: Stage) -> Duration {
            Duration::from_nanos(self.nanos[stage.index()])
        }

        /// How many times `stage` ran.
        #[must_use]
        pub fn calls(&self, stage: Stage) -> u64 {
            self.calls[stage.index()]
        }

        /// The sum of the stages inside `Batch`.
        #[must_use]
        pub fn inner_total(&self) -> Duration {
            Stage::INNER.iter().map(|s| self.total(*s)).sum()
        }

        /// What was recorded after `earlier` (saturating, per stage).
        #[must_use]
        pub fn since(&self, earlier: &StageSnapshot) -> StageSnapshot {
            StageSnapshot {
                nanos: std::array::from_fn(|i| self.nanos[i].saturating_sub(earlier.nanos[i])),
                calls: std::array::from_fn(|i| self.calls[i].saturating_sub(earlier.calls[i])),
            }
        }
    }

    /// The process-wide counters every store's writer records into.
    pub static STAGES: StageTimes = StageTimes::new();

    /// The process-wide totals so far (every store in the process).
    #[must_use]
    pub fn snapshot() -> StageSnapshot {
        STAGES.snapshot()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn stage_order_matches_index() {
            for (i, s) in Stage::ALL.iter().enumerate() {
                assert_eq!(s.index(), i, "{} out of place", s.name());
            }
            assert!(Stage::INNER.iter().all(|s| Stage::ALL.contains(s)));
            assert!(!Stage::INNER.contains(&Stage::Batch));
            assert!(!Stage::INNER.contains(&Stage::AfterCommit));
        }

        #[test]
        fn accumulator_sums_time_and_calls() {
            let t = StageTimes::new();
            t.record(Stage::Json, Duration::from_nanos(5));
            t.record(Stage::Json, Duration::from_nanos(7));
            t.record(Stage::Zstd, Duration::from_micros(3));
            let s = t.snapshot();
            assert_eq!(s.total(Stage::Json), Duration::from_nanos(12));
            assert_eq!(s.calls(Stage::Json), 2);
            assert_eq!(s.total(Stage::Zstd), Duration::from_nanos(3_000));
            assert_eq!(s.calls(Stage::Zstd), 1);
            assert_eq!(s.total(Stage::Fsync), Duration::ZERO);
            assert_eq!(s.calls(Stage::Fsync), 0);
            assert_eq!(s.inner_total(), Duration::from_nanos(3_012));
        }

        #[test]
        fn inner_total_excludes_the_batch_totals() {
            let t = StageTimes::new();
            t.record(Stage::Commit, Duration::from_nanos(10));
            t.record(Stage::Batch, Duration::from_nanos(100));
            t.record(Stage::AfterCommit, Duration::from_nanos(1_000));
            assert_eq!(t.snapshot().inner_total(), Duration::from_nanos(10));
        }

        #[test]
        fn since_subtracts_per_stage() {
            let t = StageTimes::new();
            t.record(Stage::Exists, Duration::from_nanos(4));
            let before = t.snapshot();
            t.record(Stage::Exists, Duration::from_nanos(6));
            t.record(Stage::Commit, Duration::from_nanos(9));
            let run = t.snapshot().since(&before);
            assert_eq!(run.total(Stage::Exists), Duration::from_nanos(6));
            assert_eq!(run.calls(Stage::Exists), 1);
            assert_eq!(run.total(Stage::Commit), Duration::from_nanos(9));
            // An earlier snapshot taken "after" never underflows.
            assert_eq!(before.since(&t.snapshot()), StageSnapshot::default());
        }

        #[test]
        fn huge_duration_saturates() {
            let t = StageTimes::new();
            t.record(Stage::Fsync, Duration::MAX);
            assert_eq!(
                t.snapshot().total(Stage::Fsync).as_nanos(),
                u128::from(u64::MAX)
            );
        }

        #[test]
        fn macro_times_and_returns_the_value() {
            let before = snapshot();
            let v: Result<u32, ()> = stage!(SqlBatch, Ok(41 + 1));
            assert_eq!(v, Ok(42));
            let run = snapshot().since(&before);
            assert!(run.calls(Stage::SqlBatch) >= 1);
        }
    }
}
