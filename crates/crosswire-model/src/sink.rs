//! The hand-off from the proxy to the store.

use core::fmt;
use serde::{Deserialize, Serialize};

use crate::{Exchange, Marker};

/// Why an [`ExchangeSink::submit`] or [`ExchangeSink::submit_marker`] failed.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum SinkError {
    /// The queue is full; the exchange was not accepted.
    Full,
    /// The sink is shut down.
    Closed,
}

impl fmt::Display for SinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SinkError::Full => f.write_str("exchange sink is full"),
            SinkError::Closed => f.write_str("exchange sink is closed"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for SinkError {}

/// Accepts finished exchanges. `submit` never waits for the consumer: when
/// nothing more can be accepted it returns [`SinkError::Full`] and the
/// caller records a drop. It may still block briefly on file I/O, so the
/// relay calls it off its async tasks (for example via `spawn_blocking`).
pub trait ExchangeSink: Send + Sync {
    /// Queues an exchange for recording.
    ///
    /// May block on file I/O (a store spills to a fsynced file when its
    /// queue is full), so call it from a thread that may block, for example
    /// inside `spawn_blocking` in an async runtime.
    ///
    /// # Errors
    ///
    /// [`SinkError::Full`] if the queue is at capacity, [`SinkError::Closed`]
    /// if the sink has shut down.
    fn submit(&self, exchange: Exchange) -> Result<(), SinkError>;

    /// Queues a marker row (a gap, a limit hit or an error the relay saw)
    /// for recording in the same timeline as the exchanges. The store writes
    /// the row with its next batch and then publishes
    /// [`Event::Marker`](crate::Event::Marker).
    ///
    /// # Errors
    ///
    /// [`SinkError::Full`] if the marker cannot be accepted now,
    /// [`SinkError::Closed`] if the sink has shut down.
    fn submit_marker(&self, marker: Marker) -> Result<(), SinkError>;
}
