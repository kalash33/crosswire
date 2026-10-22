//! The [`ExchangeSink`] the proxy submits to: bounded queue, journal spill.

use crate::events::Events;
use crate::journal::{Journal, SpillError};
use crosswire_model::{Body, Event, Exchange, ExchangeSink, Head, Message, SinkError};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{SyncSender, TrySendError};
use std::sync::{Arc, PoisonError, RwLock};

/// How the writer should stop.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Stop {
    /// `close`: drain, final batch, `TRUNCATE` checkpoint.
    Clean,
    /// `Drop` without `close`: drain and stop; the project stays unclean.
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
    pub(crate) queued: AtomicUsize,
    pub(crate) queued_bytes: AtomicUsize,
    /// Set by `close`/`Drop` under the write lock; `submit` holds the read
    /// lock, so nothing is enqueued after the stop message.
    pub(crate) closed: RwLock<bool>,
    pub(crate) events: Events,
    pub(crate) journal: Journal,
    /// Exchanges were dropped since the last gap marker; the writer's next
    /// commit records one.
    pub(crate) gap_pending: AtomicBool,
}

impl Shared {
    /// Reserves one queue slot and `size` bytes, or neither.
    fn reserve(&self, size: usize) -> bool {
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
        true
    }

    /// Returns what [`Shared::reserve`] took. Called once per successful
    /// reserve: by the writer when it receives the exchange, or by `submit`
    /// when the send fails.
    pub(crate) fn release(&self, size: usize) {
        self.queued.fetch_sub(1, Ordering::SeqCst);
        self.queued_bytes.fetch_sub(size, Ordering::SeqCst);
    }

    /// Notes that exchanges were dropped; broadcasts one `Error` per episode.
    pub(crate) fn dropped(&self, detail: &str) {
        if !self.gap_pending.swap(true, Ordering::SeqCst) {
            self.events.error(detail);
        }
    }

    fn spill(&self, ex: &Exchange, announce: bool) -> Result<(), SinkError> {
        let id = ex.id();
        let published = self.journal.spill(ex, || {
            if announce {
                self.events.emit(|seq| Event::Received { id, seq });
            }
        });
        match published {
            Ok(()) => Ok(()),
            Err(SpillError::CapExceeded) => {
                self.dropped("queue and journal full: recording stopped, exchanges are dropped");
                Err(SinkError::Full)
            }
            Err(SpillError::Io(e)) => {
                self.dropped(&format!(
                    "journal write failed ({e}): recording stopped, exchanges are dropped"
                ));
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
    /// Never blocks on the writer: the exchange goes to the queue if a slot
    /// and the bytes are free, otherwise to a fsynced journal file. Returns
    /// [`SinkError::Full`] only when the journal is at its cap (or cannot be
    /// written); the drop is then recorded by a gap marker.
    fn submit(&self, exchange: Exchange) -> Result<(), SinkError> {
        let shared = &*self.shared;
        let closed = shared.closed.read().unwrap_or_else(PoisonError::into_inner);
        if *closed {
            return Err(SinkError::Closed);
        }
        let size = approx_size(&exchange);
        if !shared.reserve(size) {
            return shared.spill(&exchange, true);
        }
        let id = exchange.id();
        // Announce before sending, so `Received` always precedes `Committed`.
        shared.events.emit(|seq| Event::Received { id, seq });
        match shared.tx.try_send(Msg::Exchange(Box::new(exchange), size)) {
            Ok(()) => Ok(()),
            // Unreachable while reservations bound the channel, and a writer
            // that died leaves the journal for the next open: spill either way.
            Err(TrySendError::Full(msg) | TrySendError::Disconnected(msg)) => {
                shared.release(size);
                match msg {
                    Msg::Exchange(ex, _) => shared.spill(&ex, false),
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
