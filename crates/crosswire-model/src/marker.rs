//! Gap and error markers: their own row type, not exchanges.

use alloc::string::String;
use serde::{Deserialize, Serialize};
use ulid::Ulid;

/// What a [`Marker`] records.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum MarkerKind {
    /// Exchanges may be missing in a time window.
    ///
    /// `from_ulid` and `to_ulid` bound the window by ULID timestamp: any
    /// exchange that was submitted but not recorded has an id in
    /// `from_ulid..=to_ulid` (a missing `from_ulid` means "from the start").
    /// Exchange ids are minted when the exchange is created, not when it is
    /// submitted, so an exchange that started before `from_ulid` but was
    /// submitted after it (a long-running one) can also be missing.
    Gap,
    /// A configured limit was exceeded.
    LimitExceeded,
    /// An error interrupted recording.
    Error,
    /// A kind written by a newer version of Crosswire.
    #[serde(other)]
    Unknown,
}

/// A marker row in the history timeline.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Marker {
    #[serde(with = "crate::ulid_serde")]
    id: Ulid,
    kind: MarkerKind,
    #[serde(default, with = "crate::ulid_serde::option")]
    from_ulid: Option<Ulid>,
    #[serde(default, with = "crate::ulid_serde::option")]
    to_ulid: Option<Ulid>,
    #[serde(default)]
    detail: String,
}

impl Marker {
    /// Creates a marker with a fresh ULID.
    #[cfg(feature = "std")]
    #[must_use]
    pub fn new(kind: MarkerKind, detail: impl Into<String>) -> Marker {
        Marker::with_id(Ulid::new(), kind, detail)
    }

    /// Creates a marker with the given id (for `no_std` callers and tests).
    #[must_use]
    pub fn with_id(id: Ulid, kind: MarkerKind, detail: impl Into<String>) -> Marker {
        Marker {
            id,
            kind,
            from_ulid: None,
            to_ulid: None,
            detail: detail.into(),
        }
    }

    /// Sets the first id the marker spans.
    #[must_use]
    pub fn with_from(mut self, from: Ulid) -> Marker {
        self.from_ulid = Some(from);
        self
    }

    /// Sets the last id the marker spans.
    #[must_use]
    pub fn with_to(mut self, to: Ulid) -> Marker {
        self.to_ulid = Some(to);
        self
    }

    /// The marker id.
    #[must_use]
    pub fn id(&self) -> Ulid {
        self.id
    }

    /// What the marker records.
    #[must_use]
    pub fn kind(&self) -> MarkerKind {
        self.kind
    }

    /// First id of the affected range, if known.
    #[must_use]
    pub fn from_ulid(&self) -> Option<Ulid> {
        self.from_ulid
    }

    /// Last id of the affected range, if known.
    #[must_use]
    pub fn to_ulid(&self) -> Option<Ulid> {
        self.to_ulid
    }

    /// Human-readable detail.
    #[must_use]
    pub fn detail(&self) -> &str {
        &self.detail
    }
}
