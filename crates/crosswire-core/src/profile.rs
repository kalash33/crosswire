//! Parsing profiles.

/// How strictly input is judged.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum Profile {
    /// Reject or escalate anything the RFCs forbid.
    Strict,
    /// Accept and annotate; the default.
    #[default]
    Lenient,
}
