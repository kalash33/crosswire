//! The hand-off from the proxy to the store.

use core::fmt;
use serde::{Deserialize, Serialize};

use crate::Exchange;

/// Why a [`ExchangeSink::submit`] failed.
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

/// Accepts finished exchanges. `submit` must not block the relay: a full
/// queue returns [`SinkError::Full`] and the caller records a drop.
pub trait ExchangeSink: Send + Sync {
    /// Queues an exchange for recording.
    ///
    /// # Errors
    ///
    /// [`SinkError::Full`] if the queue is at capacity, [`SinkError::Closed`]
    /// if the sink has shut down.
    fn submit(&self, exchange: Exchange) -> Result<(), SinkError>;
}
