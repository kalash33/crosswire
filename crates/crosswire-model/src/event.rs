//! Events the recorder publishes about the exchange stream.

use alloc::string::String;
use alloc::vec::Vec;
use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::Marker;

/// A recorder event. Every variant carries a monotonically increasing
/// `seq` so a consumer can detect loss; a slow consumer is told with
/// [`Event::Lagged`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
#[non_exhaustive]
pub enum Event {
    /// An exchange was accepted into the write queue.
    Received {
        /// The exchange id.
        #[serde(with = "crate::ulid_serde")]
        id: Ulid,
        /// Event sequence number (0 when absent).
        #[serde(default)]
        seq: u64,
    },
    /// A batch of exchanges was durably committed.
    Committed {
        /// The committed ids.
        #[serde(default, with = "crate::ulid_serde::vec")]
        ids: Vec<Ulid>,
        /// Event sequence number (0 when absent).
        #[serde(default)]
        seq: u64,
    },
    /// A marker row was written.
    Marker {
        /// The marker.
        marker: Marker,
        /// Event sequence number (0 when absent).
        #[serde(default)]
        seq: u64,
    },
    /// Something failed.
    Error {
        /// What failed.
        #[serde(default)]
        detail: String,
        /// Event sequence number (0 when absent).
        #[serde(default)]
        seq: u64,
    },
    /// A configured limit was exceeded.
    LimitExceeded {
        /// Which limit.
        #[serde(default)]
        detail: String,
        /// Event sequence number (0 when absent).
        #[serde(default)]
        seq: u64,
    },
    /// The consumer fell behind and events were dropped.
    Lagged {
        /// How many events were missed.
        missed: u64,
        /// Event sequence number (0 when absent).
        #[serde(default)]
        seq: u64,
    },
    /// An event written by a newer version of Crosswire.
    #[serde(other)]
    Unknown,
}

impl Event {
    /// The event's sequence number (0 for [`Event::Unknown`]).
    #[must_use]
    pub fn seq(&self) -> u64 {
        match self {
            Event::Received { seq, .. }
            | Event::Committed { seq, .. }
            | Event::Marker { seq, .. }
            | Event::Error { seq, .. }
            | Event::LimitExceeded { seq, .. }
            | Event::Lagged { seq, .. } => *seq,
            Event::Unknown => 0,
        }
    }
}
