//! The store: owns the writer thread and hands out sinks, events and readers.

use crate::audit::AuditLog;
use crate::error::StoreError;
use crate::events::{EventStream, Events};
use crate::journal::Journal;
use crate::sink::{Msg, Shared, Stop, StoreSink};
use crate::writer::Writer;
use crate::{sweep, Project};
use crosswire_model::ExchangeSink;
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
/// `Gap` marker from the last committed id.
///
/// Call [`Store::close`] to shut down cleanly. Dropping a `Store` stops the
/// writer after it commits what is queued, but leaves the project marked
/// unclean, so the next open records a gap.
#[derive(Debug)]
pub struct Store {
    shared: Arc<Shared>,
    dir: Arc<Path>,
    audit: Mutex<AuditLog>,
    writer: Option<JoinHandle<Project>>,
}

impl Store {
    /// Opens the store on `project`.
    ///
    /// Runs the startup sweep, commits every file left in `journal/`, writes
    /// a `Gap` marker if the previous run did not close cleanly, records
    /// `clean=0` in `project.lock`, and starts the writer thread.
    pub fn open(mut project: Project, cfg: StoreConfig) -> Result<Store, StoreError> {
        sweep(&mut project)?;
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
            closed: RwLock::new(false),
            events: Events::new(cfg.event_capacity),
            journal,
            gap_pending: AtomicBool::new(false),
        });
        let unclean = project.previous_unclean();
        let mut writer = Writer::new(project, Arc::clone(&shared), cfg);
        // Always: replay is idempotent, and a clean run leaves nothing to do.
        writer.drain_journal();
        if unclean {
            writer.mark_unclean_restart()?;
        }
        writer.mark_dirty()?;
        let handle = std::thread::Builder::new()
            .name("crosswire-store-writer".into())
            .spawn(move || writer.run(&rx))?;
        Ok(Store {
            shared,
            dir,
            audit: Mutex::new(audit),
            writer: Some(handle),
        })
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
    /// If losses happened that could not be recorded with a gap marker, the
    /// project is left unclean (so the next open records one) and this
    /// returns [`StoreError::Writer`].
    pub fn close(mut self) -> Result<(), StoreError> {
        let project = self.stop(Stop::Clean)?;
        if self.shared.gap_pending.load(Ordering::SeqCst) {
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
        handle
            .join()
            .map_err(|_| StoreError::Writer("the writer thread panicked".into()))
    }
}

impl Drop for Store {
    /// Stops the writer without recording a clean shutdown.
    fn drop(&mut self) {
        if self.writer.is_some() {
            let _ = self.stop(Stop::Abandon);
        }
    }
}
