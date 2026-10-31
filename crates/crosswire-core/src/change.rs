//! Records of modifications made to a message.

use crate::Span;
use alloc::string::String;
use bytes::Bytes;

/// The kind of edit.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "kebab-case"))]
#[non_exhaustive]
pub enum ChangeOp {
    /// Bytes were inserted.
    Insert,
    /// Bytes were removed.
    Delete,
    /// Bytes were replaced.
    Replace,
    /// An edit kind written by a newer version of Crosswire.
    #[cfg_attr(feature = "serde", serde(other))]
    Unknown,
}

/// Why an edit was made.
///
/// On the wire the unit reasons are kebab-case strings and the data reasons
/// are externally tagged objects (`{"intruder-position": 3}`). Reading is
/// hand-written so that any unrecognised code, bare or carrying a payload
/// (`{"rule-group": {...}}`), reads as [`ChangeReason::Unknown`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "kebab-case"))]
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
    /// A reason written by a newer version of Crosswire (an unrecognised
    /// code, with or without a payload; the payload is skipped).
    Unknown,
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for ChangeReason {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<ChangeReason, D::Error> {
        use core::fmt;
        use serde::de::{self, IgnoredAny, MapAccess, Visitor};

        /// The `match-replace` payload, as the derived `Serialize` writes it.
        #[derive(serde::Deserialize)]
        struct MatchReplace {
            rule: String,
        }

        fn unit(code: &str) -> ChangeReason {
            match code {
                "hop-by-hop" => ChangeReason::HopByHop,
                "content-length" => ChangeReason::ContentLength,
                "intercept-edit" => ChangeReason::InterceptEdit,
                _ => ChangeReason::Unknown,
            }
        }

        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = ChangeReason;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a change reason code or a single-key object")
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<ChangeReason, E> {
                Ok(unit(v))
            }

            fn visit_map<A: MapAccess<'de>>(self, mut m: A) -> Result<ChangeReason, A::Error> {
                let Some(key) = m.next_key::<String>()? else {
                    return Ok(ChangeReason::Unknown);
                };
                let reason = match key.as_str() {
                    "match-replace" => ChangeReason::MatchReplace {
                        rule: m.next_value::<MatchReplace>()?.rule,
                    },
                    "intruder-position" => ChangeReason::IntruderPosition(m.next_value()?),
                    "custom" => ChangeReason::Custom(m.next_value()?),
                    other => {
                        m.next_value::<IgnoredAny>()?;
                        unit(other)
                    }
                };
                // Entries after the first (none are written) are skipped.
                while m.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
                Ok(reason)
            }
        }

        d.deserialize_any(V)
    }
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
