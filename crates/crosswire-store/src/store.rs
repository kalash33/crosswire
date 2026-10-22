//! The store: owns the writer thread and hands out sinks, events and readers.

use crate::audit::AuditLog;
use crate::error::StoreError;
use crate::events::{EventStream, Events};
use crate::journal::Journal;
use crate::sink::{Msg, Shared, Stop, StoreSink};
use crate::writer::Writer;
use crate::{sweep, Project, SweepReport};
use crosswire_model::{Event, ExchangeSink, Marker};
use rusqlite::Connection;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::sync_channel;
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::thread::JoinHandle;
use std::time::Duration;

const MIB: usize = 1024 * 1024;

/// Writer, queue and journal settings. The defaults are the limits the
/// durability promise is stated in (spec section 9.3).
#[derive(Clone, Debug)]
pub struct StoreConfig {
    /// Longest wait between the first exchange of a batch and its commit (250 ms).
    pub batch_interval: Duration,
    /// Most exchanges per batch (1,000).
    pub batch_max: usize,
    /// Most exchanges waiting in the queue (10,000).
    pub queue_max: usize,
    /// Most bytes waiting in the queue (64 MiB); an exchange costs its
    /// message bytes plus 1 KiB.
    pub queue_bytes_max: usize,
    /// Longest time between WAL checkpoints (30 s).
    pub checkpoint_interval: Duration,
    /// WAL size that triggers a checkpoint (64 MiB).
    pub checkpoint_bytes: u64,
    /// Most bytes of spilled exchanges in `journal/` (1 GiB). Past it,
    /// exchanges are dropped and a gap marker is written.
    pub journal_cap: u64,
    /// Events buffered per subscriber before it lags (10,000).
    pub event_capacity: usize,
    /// Size at which `audit.jsonl` rotates to `audit.jsonl.1` (64 MiB).
    pub audit_rotate_bytes: u64,
    /// Test hook: the writer holds this lock while it receives and commits
    /// each batch, so a test can stall it.
    #[doc(hidden)]
    pub test_pause: Option<Arc<Mutex<()>>>,
    /// Fault injection for the crate's own tests.
    #[cfg(test)]
    pub(crate) hooks: Arc<TestHooks>,
}

/// Fault injection for unit tests.
#[cfg(test)]
#[derive(Debug, Default)]
pub(crate) struct TestHooks {
    /// The next this many commits fail with `SQLITE_FULL` just before `COMMIT`.
    pub(crate) fail_commits: std::sync::atomic::AtomicUsize,
    /// Storing this exchange fails as that exchange's fault.
    pub(crate) fail_item: Mutex<Option<ulid::Ulid>>,
    /// The writer thread panics on its next non-empty batch.
    pub(crate) panic_next_batch: AtomicBool,
}

/// What [`Store::open`] did before the writer started.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct OpenReport {
    /// The startup sweep.
    pub sweep: SweepReport,
    /// Journal files that left `journal/` (committed or quarantined).
    pub journal_files_drained: usize,
    /// The gap marker written because the previous run did not close.
    pub unclean_gap: Option<Marker>,
    /// Every event sent during open (markers, errors such as quarantined
    /// journal files, commits of replayed exchanges), sent before anyone
    /// could subscribe.
    pub events: Vec<Event>,
}

impl Default for StoreConfig {
    fn default() -> StoreConfig {
        StoreConfig {
            batch_interval: Duration::from_millis(250),
            batch_max: 1000,
            queue_max: 10_000,
            queue_bytes_max: 64 * MIB,
            checkpoint_interval: Duration::from_secs(30),
            checkpoint_bytes: 64 * MIB as u64,
            journal_cap: 1024 * MIB as u64,
            event_capacity: 10_000,
            audit_rotate_bytes: 64 * MIB as u64,
            test_pause: None,
            #[cfg(test)]
            hooks: Arc::default(),
        }
    }
}

/// An open store: a project whose writer connection lives on its own thread.
///
/// The durability promise: an exchange whose [`crosswire_model::Event::Committed`]
/// was broadcast survives a process crash. A crash loses at most what was
/// queued and not yet committed (one batch when the writer keeps up, at most
/// the bounded queue otherwise); spilled exchanges in `journal/` survive and
/// are committed on the next open. The next open after a crash writes a
/// `Gap` marker whose window (by ULID time) covers every exchange that was
/// accepted and lost; see [`crosswire_model::MarkerKind::Gap`].
///
/// Call [`Store::close`] to shut down cleanly. Dropping a `Store` stops the
/// writer after it commits what is in memory, but leaves the project marked
/// unclean, so the next open records a gap.
#[derive(Debug)]
pub struct Store {
    shared: Arc<Shared>,
    dir: Arc<Path>,
    audit: Mutex<AuditLog>,
    writer: Option<JoinHandle<Project>>,
    report: OpenReport,
    /// Idle read-only connections (never a writer), at most four.
    pub(crate) readers: Mutex<Vec<Connection>>,
}

impl Store {
    /// Opens the store on `project`.
    ///
    /// Runs the startup sweep; if the previous run did not close cleanly,
    /// writes a `Gap` marker from the last commit's watermark to now (read
    /// before anything else is committed); commits every file left in
    /// `journal/`; records `clean=0` in `project.lock`; and starts the writer
    /// thread. What happened is in [`Store::open_report`].
    pub fn open(mut project: Project, cfg: StoreConfig) -> Result<Store, StoreError> {
        let sweep_report = sweep(&mut project)?;
        // Keep the WAL from staying at its high-water size, so its length
        // tracks what is not yet checkpointed.
        project
            .writer_mut()
            .pragma_update(None, "journal_size_limit", 1024 * 1024)?;
        let dir: Arc<Path> = Arc::from(project.dir());
        let journal = Journal::open(&dir, cfg.journal_cap)?;
        let audit = AuditLog::open(&dir, cfg.audit_rotate_bytes)?;
        let queue_max = cfg.queue_max.max(1);
        let (tx, rx) = sync_channel(queue_max);
        let shared = Arc::new(Shared {
            tx,
            queue_max,
            queue_bytes_max: cfg.queue_bytes_max,
            queued: AtomicUsize::new(0),
            queued_bytes: AtomicUsize::new(0),
            in_memory: Mutex::default(),
            closed: RwLock::new(false),
            events: Events::new(cfg.event_capacity),
            journal,
            gap_since: Mutex::new(None),
            writer_dead: AtomicBool::new(false),
        });
        shared.events.start_capture();
        let unclean = project.previous_unclean();
        let mut writer = Writer::new(project, Arc::clone(&shared), cfg);
        let unclean_gap = if unclean {
            Some(writer.mark_unclean_restart()?)
        } else {
            None
        };
        // Always: replay is idempotent, and a clean run leaves nothing to do.
        let journal_files_drained = writer.drain_journal();
        writer.mark_dirty()?;
        let report = OpenReport {
            sweep: sweep_report,
            journal_files_drained,
            unclean_gap,
            events: shared.events.take_capture(),
        };
        let handle = std::thread::Builder::new()
            .name("crosswire-store-writer".into())
            .spawn(move || writer.run(&rx))?;
        Ok(Store {
            shared,
            dir,
            audit: Mutex::new(audit),
            writer: Some(handle),
            report,
            readers: Mutex::new(Vec::new()),
        })
    }

    /// What [`Store::open`] did: sweep, replay, unclean-restart marker, and
    /// the events sent before anyone could subscribe.
    #[must_use]
    pub fn open_report(&self) -> &OpenReport {
        &self.report
    }

    /// Whether the writer thread is running. False once it stopped
    /// abnormally (the sink then refuses with [`crosswire_model::SinkError::Closed`])
    /// or the store is shutting down.
    #[must_use]
    pub fn is_healthy(&self) -> bool {
        !self.shared.writer_dead.load(Ordering::SeqCst)
            && self.writer.as_ref().is_some_and(|h| !h.is_finished())
    }

    /// The project directory.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The sink the proxy submits finished exchanges to.
    #[must_use]
    pub fn sink(&self) -> Arc<dyn ExchangeSink> {
        Arc::new(StoreSink {
            shared: Arc::clone(&self.shared),
        })
    }

    /// Subscribes to events from now on (see [`EventStream`]).
    #[must_use]
    pub fn subscribe(&self) -> EventStream {
        self.shared.events.subscribe()
    }

    /// A new read-only connection (see [`Project::reader`]).
    pub fn reader(&self) -> Result<Connection, StoreError> {
        crate::project::open_reader(&self.dir)
    }

    /// Appends one row to `audit.jsonl` and syncs it before returning.
    ///
    /// The row is a JSON object with `ts` (unix milliseconds), `ulid`,
    /// `actor`, `action`, `target` and `result`. At 64 MiB (by default) the
    /// log is renamed to `audit.jsonl.1`, replacing the previous generation.
    pub fn audit(
        &self,
        actor: &str,
        action: &str,
        target: &str,
        result: &str,
    ) -> Result<(), StoreError> {
        self.audit
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .append(actor, action, target, result)
    }

    /// Shuts down cleanly: refuses new exchanges, commits the queue and the
    /// journal, checkpoints and truncates the WAL, and records `clean=1`.
    ///
    /// If the writer thread stopped abnormally, or losses happened that
    /// could not be recorded with a gap marker, the project is left unclean
    /// (so the next open records a gap) and this returns
    /// [`StoreError::Writer`] with the reason.
    pub fn close(mut self) -> Result<(), StoreError> {
        let project = self.stop(Stop::Clean)?;
        if self.shared.gap_pending() {
            return Err(StoreError::Writer(
                "dropped exchanges could not be marked; project left unclean".into(),
            ));
        }
        project.mark_clean()
    }

    fn stop(&mut self, how: Stop) -> Result<Project, StoreError> {
        *self
            .shared
            .closed
            .write()
            .unwrap_or_else(PoisonError::into_inner) = true;
        let handle = self.writer.take().ok_or(StoreError::Closed)?;
        // Fails only if the writer is gone; the join below reports why.
        let _ = self.shared.tx.send(Msg::Stop(how));
        handle.join().map_err(|panic| {
            let why = panic
                .downcast_ref::<&str>()
                .map(|s| (*s).to_owned())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_default();
            StoreError::Writer(format!("the writer thread panicked: {why}"))
        })
    }
}

impl Drop for Store {
    /// Stops the writer without recording a clean shutdown. It commits what
    /// is in memory (at most the bounded queue) and returns; `journal/` is
    /// left for the next open, so this never waits on a large journal.
    fn drop(&mut self) {
        if self.writer.is_some() {
            let _ = self.stop(Stop::Abandon);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crosswire_model::{
        Body, ConnectionInfo, Exchange, Fidelity, FramingRecord, Head, Message, MessageKind,
        SinkError,
    };
    use std::collections::BTreeSet;
    use std::time::Instant;
    use ulid::Ulid;

    fn exchange(body: &[u8]) -> Exchange {
        let msg = Message::new(
            MessageKind::Normalized,
            Fidelity::Normalized,
            Head::Normalized { map: Vec::new() },
            FramingRecord::None,
            Body::Inline {
                bytes: bytes::Bytes::copy_from_slice(body),
            },
            ConnectionInfo::new("example.test", 443),
        );
        Exchange::new(msg)
    }

    fn open(tmp: &tempfile::TempDir) -> (StoreConfig, Store) {
        let cfg = StoreConfig {
            batch_interval: Duration::from_millis(20),
            ..StoreConfig::default()
        };
        let p = Project::create(&tmp.path().join("p")).unwrap();
        (cfg.clone(), Store::open(p, cfg).unwrap())
    }

    /// Drains `ev` until `limit`, returning committed ids and error details.
    fn watch(
        ev: &mut EventStream,
        want: &[Ulid],
        limit: Duration,
    ) -> (BTreeSet<Ulid>, Vec<String>) {
        let deadline = Instant::now() + limit;
        let (mut got, mut errors) = (BTreeSet::new(), Vec::new());
        while Instant::now() < deadline && !want.iter().all(|w| got.contains(w)) {
            match ev.try_recv() {
                Some(Event::Committed { ids, .. }) => got.extend(ids),
                Some(Event::Error { detail, .. }) => errors.push(detail),
                Some(_) => {}
                None => std::thread::sleep(Duration::from_millis(2)),
            }
        }
        (got, errors)
    }

    fn bad_files(store: &Store) -> usize {
        std::fs::read_dir(store.dir().join("journal").join("bad")).map_or(0, Iterator::count)
    }

    #[test]
    fn database_failure_keeps_exchanges_and_recovers() {
        let tmp = tempfile::tempdir().unwrap();
        let (cfg, store) = open(&tmp);
        let mut ev = store.subscribe();
        // Three failed attempts: 250 + 500 + 1000 ms of backoff.
        cfg.hooks.fail_commits.store(3, Ordering::SeqCst);
        let exs: Vec<Exchange> = (0..5u8).map(|i| exchange(&[i; 100])).collect();
        let ids: Vec<Ulid> = exs.iter().map(Exchange::id).collect();
        for ex in exs {
            store.sink().submit(ex).unwrap();
        }
        let (got, errors) = watch(&mut ev, &ids, Duration::from_secs(15));
        assert!(
            ids.iter().all(|i| got.contains(i)),
            "all committed after recovery"
        );
        assert_eq!(errors.len(), 1, "one Error per failure streak: {errors:?}");
        assert_eq!(
            bad_files(&store),
            0,
            "nothing quarantined for a database failure"
        );
        assert!(store.is_healthy());
        let dir = store.dir().to_path_buf();
        store.close().unwrap();
        let journal = std::fs::read_dir(dir.join("journal"))
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .path()
                    .extension()
                    .is_some_and(|x| x == "json")
            })
            .count();
        assert_eq!(journal, 0, "written-back exchanges were drained");
        let conn = crate::project::open_reader(&dir).unwrap();
        let gaps: i64 = conn
            .query_row("SELECT count(*) FROM markers", [], |r| r.get(0))
            .unwrap();
        assert_eq!(gaps, 0, "nothing was lost");
    }

    #[test]
    fn failing_exchange_is_set_aside_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let (cfg, store) = open(&tmp);
        let mut ev = store.subscribe();
        let exs: Vec<Exchange> = (0..3u8).map(|i| exchange(&[i; 10])).collect();
        let ids: Vec<Ulid> = exs.iter().map(Exchange::id).collect();
        *cfg.hooks.fail_item.lock().unwrap() = Some(ids[1]);
        for ex in exs {
            store.sink().submit(ex).unwrap();
        }
        let want = [ids[0], ids[2]];
        let (got, errors) = watch(&mut ev, &want, Duration::from_secs(5));
        assert!(want.iter().all(|i| got.contains(i)));
        assert!(!got.contains(&ids[1]));
        assert!(errors.iter().any(|e| e.contains(&ids[1].to_string())));
        assert!(store
            .dir()
            .join("journal")
            .join("bad")
            .join(format!("{}.json", ids[1]))
            .is_file());
        let dir = store.dir().to_path_buf();
        store.close().unwrap();
        let conn = crate::project::open_reader(&dir).unwrap();
        let (from, to): (String, String) = conn
            .query_row(
                "SELECT from_ulid, to_ulid FROM markers WHERE kind = 'gap'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        let (from, to) = (
            Ulid::from_string(&from).unwrap(),
            Ulid::from_string(&to).unwrap(),
        );
        assert!(
            from <= ids[1] && ids[1] <= to,
            "the gap brackets the set-aside exchange"
        );
    }

    #[test]
    fn dead_writer_is_reported_and_refuses() {
        let tmp = tempfile::tempdir().unwrap();
        let (cfg, store) = open(&tmp);
        let mut ev = store.subscribe();
        cfg.hooks.panic_next_batch.store(true, Ordering::SeqCst);
        store.sink().submit(exchange(b"boom")).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut stopped = Vec::new();
        while Instant::now() < deadline && stopped.is_empty() {
            match ev.try_recv() {
                Some(Event::Error { detail, .. }) if detail.starts_with("writer stopped") => {
                    stopped.push(detail);
                }
                Some(_) => {}
                None => std::thread::sleep(Duration::from_millis(2)),
            }
        }
        assert_eq!(stopped.len(), 1);
        assert!(!store.is_healthy());
        assert_eq!(store.sink().submit(exchange(b"x")), Err(SinkError::Closed));
        assert_eq!(store.sink().submit(exchange(b"y")), Err(SinkError::Closed));
        while let Some(e) = ev.try_recv() {
            assert!(
                !matches!(&e, Event::Error { detail, .. } if detail.starts_with("writer stopped")),
                "only one writer-stopped Error"
            );
        }
        let dir = store.dir().to_path_buf();
        match store.close() {
            Err(StoreError::Writer(why)) => assert!(why.contains("injected writer panic")),
            other => panic!("expected a writer error, got {other:?}"),
        }
        let lock = std::fs::read_to_string(dir.join("project.lock")).unwrap();
        assert!(lock.contains("clean=0"));
    }
}
