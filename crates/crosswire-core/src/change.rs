//! Records of modifications made to a message.

use crate::Span;
use alloc::string::String;
use bytes::Bytes;

/// The kind of edit.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum ChangeOp {
    /// Bytes were inserted.
    Insert,
    /// Bytes were removed.
    Delete,
    /// Bytes were replaced.
    Replace,
}

/// Why an edit was made.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum ChangeReason {
    /// A hop-by-hop header was removed.
    HopByHop,
    /// `Content-Length` was recomputed.
    ContentLength,
    /// A match/replace rule fired.
    MatchReplace {
        /// Name of the rule.
        rule: String,
    },
    /// An Intruder payload position was filled.
    IntruderPosition(u16),
    /// The user edited the message while intercepting.
    InterceptEdit,
    /// Any other reason.
    Custom(String),
}

/// A single edit applied to a message.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Change {
    op: ChangeOp,
    span: Span,
    before: Bytes,
    after: Bytes,
    reason: ChangeReason,
}

impl Change {
    /// Creates a change record.
    #[must_use]
    pub fn new(
        op: ChangeOp,
        span: Span,
        before: Bytes,
        after: Bytes,
        reason: ChangeReason,
    ) -> Change {
        Change {
            op,
            span,
            before,
            after,
            reason,
        }
    }

    /// The kind of edit.
    #[must_use]
    pub fn op(&self) -> ChangeOp {
        self.op
    }

    /// The affected span in the original message.
    #[must_use]
    pub fn span(&self) -> Span {
        self.span
    }

    /// Bytes before the edit.
    #[must_use]
    pub fn before(&self) -> &Bytes {
        &self.before
    }

    /// Bytes after the edit.
    #[must_use]
    pub fn after(&self) -> &Bytes {
        &self.after
    }

    /// Why the edit was made.
    #[must_use]
    pub fn reason(&self) -> &ChangeReason {
        &self.reason
    }
}
