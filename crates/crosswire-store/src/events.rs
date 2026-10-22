//! The event broadcaster and the subscriber stream.

use crosswire_model::Event;
use std::sync::{Mutex, MutexGuard, PoisonError};
use tokio::sync::broadcast::{
    self,
    error::{RecvError, TryRecvError},
};

/// Hands every event the next sequence id and broadcasts it.
#[derive(Debug)]
pub(crate) struct Events {
    tx: broadcast::Sender<Event>,
    /// The next sequence id. A mutex rather than an atomic so that ids reach
    /// the channel in order: a subscriber sees strictly increasing ids, and a
    /// jump means it missed events. Holding it (see [`Events::hold`]) also
    /// orders a caller's action before any event another thread sends.
    next_seq: Mutex<u64>,
    /// While `Some`, every event is also recorded here (used during open,
    /// before anyone can subscribe).
    capture: Mutex<Option<Vec<Event>>>,
}

/// The sequence lock, held: no other event can be sent until it is dropped.
pub(crate) struct EventsGuard<'a> {
    events: &'a Events,
    seq: MutexGuard<'a, u64>,
}

impl EventsGuard<'_> {
    /// Builds the event with the next sequence id and broadcasts it.
    pub(crate) fn emit(&mut self, make: impl FnOnce(u64) -> Event) {
        let event = make(*self.seq);
        *self.seq += 1;
        if let Some(v) = self
            .events
            .capture
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_mut()
        {
            v.push(event.clone());
        }
        // No subscribers is not an error.
        let _ = self.events.tx.send(event);
    }
}

impl Events {
    /// A broadcaster that keeps up to `capacity` events per subscriber.
    pub(crate) fn new(capacity: usize) -> Events {
        let (tx, _) = broadcast::channel(capacity.clamp(1, usize::MAX / 4));
        Events {
            tx,
            next_seq: Mutex::new(1),
            capture: Mutex::new(None),
        }
    }

    /// Takes the sequence lock. Never call [`Events::emit`] while holding it.
    pub(crate) fn hold(&self) -> EventsGuard<'_> {
        EventsGuard {
            events: self,
            seq: self.next_seq.lock().unwrap_or_else(PoisonError::into_inner),
        }
    }

    /// Builds the event with the next sequence id and broadcasts it.
    pub(crate) fn emit(&self, make: impl FnOnce(u64) -> Event) {
        self.hold().emit(make);
    }

    /// Broadcasts an [`Event::Error`].
    pub(crate) fn error(&self, detail: impl Into<String>) {
        let detail = detail.into();
        self.emit(|seq| Event::Error { detail, seq });
    }

    /// Starts recording every event sent.
    pub(crate) fn start_capture(&self) {
        *self.capture.lock().unwrap_or_else(PoisonError::into_inner) = Some(Vec::new());
    }

    /// Stops recording and returns what was recorded.
    pub(crate) fn take_capture(&self) -> Vec<Event> {
        self.capture
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .unwrap_or_default()
    }

    pub(crate) fn subscribe(&self) -> EventStream {
        EventStream {
            rx: self.tx.subscribe(),
            pending: None,
            last_seq: None,
        }
    }
}

/// A subscription to the store's events, starting with the next event sent.
///
/// Every event carries a sequence id, strictly increasing. A subscriber that
/// falls more than the configured capacity (10,000 by default) behind loses
/// the oldest events and receives one [`Event::Lagged`] in their place:
/// `missed` is how many were lost and `seq` is the id of the last one lost,
/// so the event after it has id `seq + 1`.
#[derive(Debug)]
pub struct EventStream {
    rx: broadcast::Receiver<Event>,
    pending: Option<Event>,
    last_seq: Option<u64>,
}

impl EventStream {
    /// Waits for the next event; `None` once the store is gone and every
    /// buffered event was read.
    pub async fn recv(&mut self) -> Option<Event> {
        if let Some(e) = self.pending.take() {
            return Some(self.seen(e));
        }
        match self.rx.recv().await {
            Ok(e) => Some(self.seen(e)),
            Err(RecvError::Closed) => None,
            Err(RecvError::Lagged(n)) => Some(self.lagged(n)),
        }
    }

    /// As [`EventStream::recv`], blocking the thread.
    ///
    /// # Panics
    ///
    /// When called inside an asynchronous runtime (tokio's rule).
    pub fn blocking_recv(&mut self) -> Option<Event> {
        if let Some(e) = self.pending.take() {
            return Some(self.seen(e));
        }
        match self.rx.blocking_recv() {
            Ok(e) => Some(self.seen(e)),
            Err(RecvError::Closed) => None,
            Err(RecvError::Lagged(n)) => Some(self.lagged(n)),
        }
    }

    /// The next event if one is buffered; `None` if there is none yet or the
    /// store is gone.
    pub fn try_recv(&mut self) -> Option<Event> {
        if let Some(e) = self.pending.take() {
            return Some(self.seen(e));
        }
        match self.rx.try_recv() {
            Ok(e) => Some(self.seen(e)),
            Err(TryRecvError::Empty | TryRecvError::Closed) => None,
            Err(TryRecvError::Lagged(n)) => Some(self.lagged(n)),
        }
    }

    fn seen(&mut self, e: Event) -> Event {
        self.last_seq = Some(e.seq());
        e
    }

    /// Synthesises the `Lagged` event. After a lag the receiver points at the
    /// oldest retained event, which is buffered here so the `Lagged` event can
    /// carry the id just before it.
    fn lagged(&mut self, mut missed: u64) -> Event {
        loop {
            match self.rx.try_recv() {
                Ok(next) => {
                    let seq = next.seq().saturating_sub(1);
                    self.pending = Some(next);
                    self.last_seq = Some(seq);
                    return Event::Lagged { missed, seq };
                }
                // More events overflowed while we looked: count them too.
                Err(TryRecvError::Lagged(n)) => missed += n,
                Err(TryRecvError::Empty | TryRecvError::Closed) => {
                    let seq = self.last_seq.map_or(missed, |s| s + missed);
                    self.last_seq = Some(seq);
                    return Event::Lagged { missed, seq };
                }
            }
        }
    }
}
