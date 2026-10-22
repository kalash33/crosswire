//! The [`ExchangeSink`] the proxy submits to: bounded queue, journal spill.

use crate::events::Events;
use crate::journal::{Journal, SpillError};
use crosswire_model::{Body, Event, Exchange, ExchangeSink, Head, Message, SinkError};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{SyncSender, TrySendError};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};
use ulid::Ulid;

/// How far before "now" a gap window starts, so that an exchange created
/// shortly before it was submitted still falls inside the window.
pub(crate) const GAP_SLACK_MS: u64 = 1000;

/// How the writer should stop.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Stop {
    /// `close`: drain the queue and the journal, final batch, `TRUNCATE` checkpoint.
    Clean,
    /// `Drop` without `close`: drain the in-memory queue only; the journal
    /// waits for the next open and the project stays unclean.
    Abandon,
}

/// A message to the writer thread.
#[derive(Debug)]
pub(crate) enum Msg {
    /// An exchange and the queue bytes it reserved.
    Exchange(Box<Exchange>, usize),
    /// Stop after draining.
    Stop(Stop),
}

/// State shared by the store handle, its sinks and the writer thread.
#[derive(Debug)]
pub(crate) struct Shared {
    pub(crate) tx: SyncSender<Msg>,
    pub(crate) queue_max: usize,
    pub(crate) queue_bytes_max: usize,
    /// Exchanges accepted into memory and not yet durable (in the channel or
    /// held by the writer), and their reserved bytes.
    pub(crate) queued: AtomicUsize,
    pub(crate) queued_bytes: AtomicUsize,
    /// The ids counted in `queued` (a multiset); its minimum bounds what a
    /// crash can lose.
    pub(crate) in_memory: Mutex<BTreeMap<Ulid, usize>>,
    /// Set by `close`/`Drop` under the write lock; `submit` holds the read
    /// lock, so nothing is enqueued after the stop message.
    pub(crate) closed: RwLock<bool>,
    pub(crate) events: Events,
    pub(crate) journal: Journal,
    /// Start of the window in which exchanges were dropped since the last gap
    /// marker; the writer's next commit records the marker.
    pub(crate) gap_since: Mutex<Option<Ulid>>,
    /// The writer thread is gone; `submit` refuses.
    pub(crate) writer_dead: AtomicBool,
}

/// The ULID for `now - GAP_SLACK_MS` (the earliest id a gap starting now must cover).
pub(crate) fn slack_ulid() -> Ulid {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
    Ulid::from_parts(now.saturating_sub(GAP_SLACK_MS), 0)
}

impl Shared {
    /// Reserves one queue slot and `size` bytes for `id`, or nothing.
    fn reserve(&self, id: Ulid, size: usize) -> bool {
        if self
            .queued
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |c| {
                (c < self.queue_max).then_some(c + 1)
            })
            .is_err()
        {
            return false;
        }
        if self
            .queued_bytes
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |b| {
                b.checked_add(size).filter(|n| *n <= self.queue_bytes_max)
            })
            .is_err()
        {
            self.queued.fetch_sub(1, Ordering::SeqCst);
            return false;
        }
        *self
            .in_memory
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(id)
            .or_insert(0) += 1;
        true
    }

    /// Returns what [`Shared::reserve`] took, once the exchange is durable
    /// (committed or written to the journal), given up, or was never sent.
    pub(crate) fn release(&self, id: Ulid, size: usize) {
        let mut ids = self
            .in_memory
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(n) = ids.get_mut(&id) {
            *n -= 1;
            if *n == 0 {
                ids.remove(&id);
            }
        }
        drop(ids);
        self.queued.fetch_sub(1, Ordering::SeqCst);
        self.queued_bytes.fetch_sub(size, Ordering::SeqCst);
    }

    /// The oldest exchange id held in memory, skipping `except`.
    pub(crate) fn oldest_in_memory(
        &self,
        except: &std::collections::HashSet<Ulid>,
    ) -> Option<Ulid> {
        self.in_memory
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .keys()
            .find(|id| !except.contains(id))
            .copied()
    }

    /// Notes that exchanges with ids from `from` on were lost; broadcasts one
    /// `Error` per episode (until the writer records the gap marker).
    pub(crate) fn dropped(&self, from: Ulid, detail: &str) {
        let mut since = self
            .gap_since
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let first = since.is_none();
        *since = Some(since.map_or(from, |s| s.min(from)));
        drop(since);
        if first {
            self.events.error(detail);
        }
    }

    /// Takes the pending gap window start, if any.
    pub(crate) fn take_gap(&self) -> Option<Ulid> {
        self.gap_since
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }

    /// Puts back a gap that could not be recorded (without a new `Error`).
    pub(crate) fn restore_gap(&self, from: Ulid) {
        let mut since = self
            .gap_since
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        *since = Some(since.map_or(from, |s| s.min(from)));
    }

    /// Whether a loss is waiting for its gap marker.
    pub(crate) fn gap_pending(&self) -> bool {
        self.gap_since
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some()
    }

    /// Records that the writer thread stopped; one `Error` event.
    pub(crate) fn writer_died(&self, why: &str) {
        if !self.writer_dead.swap(true, Ordering::SeqCst) {
            self.events.error(format!("writer stopped: {why}"));
        }
    }

    fn spill(&self, ex: &Exchange) -> Result<(), SinkError> {
        let id = ex.id();
        let published = self.journal.prepare(ex, true).and_then(|p| {
            // Hold the sequence lock from the rename until `Received` is
            // sent: the writer cannot announce the file committed before.
            let mut events = self.events.hold();
            p.publish()?;
            events.emit(|seq| Event::Received { id, seq });
            Ok(())
        });
        match published {
            Ok(()) => Ok(()),
            Err(SpillError::CapExceeded) => {
                self.dropped(
                    id.min(slack_ulid()),
                    "queue and journal full: recording stopped, exchanges are dropped",
                );
                Err(SinkError::Full)
            }
            Err(SpillError::Io(e)) => {
                self.dropped(
                    id.min(slack_ulid()),
                    &format!(
                        "journal write failed ({e}): recording stopped, exchanges are dropped"
                    ),
                );
                Err(SinkError::Full)
            }
        }
    }
}

/// The sink returned by [`crate::Store::sink`].
#[derive(Debug)]
pub(crate) struct StoreSink {
    pub(crate) shared: Arc<Shared>,
}

impl ExchangeSink for StoreSink {
    /// The exchange goes to the queue if a slot and the bytes are free,
    /// otherwise to a fsynced journal file, so this may block on file I/O.
    /// `Received` is sent once the exchange is queued or durably spilled.
    /// Returns [`SinkError::Full`] when the journal is at its cap or cannot
    /// be written (the drop is recorded by a gap marker), and
    /// [`SinkError::Closed`] once the store is closed or its writer stopped.
    fn submit(&self, exchange: Exchange) -> Result<(), SinkError> {
        let shared = &*self.shared;
        let closed = shared.closed.read().unwrap_or_else(PoisonError::into_inner);
        if *closed || shared.writer_dead.load(Ordering::SeqCst) {
            return Err(SinkError::Closed);
        }
        let id = exchange.id();
        let size = approx_size(&exchange);
        if !shared.reserve(id, size) {
            return shared.spill(&exchange);
        }
        // Send and announce under the sequence lock, so `Received` always
        // precedes the writer's `Committed`.
        let mut events = shared.events.hold();
        match shared.tx.try_send(Msg::Exchange(Box::new(exchange), size)) {
            Ok(()) => {
                events.emit(|seq| Event::Received { id, seq });
                Ok(())
            }
            Err(TrySendError::Disconnected(_)) => {
                drop(events);
                shared.release(id, size);
                shared.writer_died("the writer thread is gone");
                Err(SinkError::Closed)
            }
            // Unreachable while reservations bound the channel.
            Err(TrySendError::Full(msg)) => {
                drop(events);
                shared.release(id, size);
                match msg {
                    Msg::Exchange(ex, _) => shared.spill(&ex),
                    Msg::Stop(_) => Err(SinkError::Closed),
                }
            }
        }
    }
}

/// An exchange's queue cost: its message bytes plus 1 KiB.
fn approx_size(ex: &Exchange) -> usize {
    let msg = |m: &Message| {
        let head = match m.head() {
            Head::Http1Raw { bytes, .. } => bytes.len(),
            Head::Normalized { map } => map.iter().map(|(k, v)| k.len() + v.len()).sum(),
            _ => 0,
        };
        let body = match m.body() {
            Body::Inline { bytes } => bytes.len(),
            _ => 0,
        };
        head + body
    };
    1024 + msg(ex.request()) + ex.response().map_or(0, msg)
}
