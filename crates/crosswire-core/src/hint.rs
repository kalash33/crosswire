//! Reserved encoding hints (no behaviour yet).

/// How a header field was represented on the wire (reserved for HPACK/QPACK).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "kebab-case"))]
#[non_exhaustive]
pub enum EncodingHint {
    /// Fully indexed field.
    Indexed,
    /// Literal field added to the dynamic table.
    LiteralWithIndexing,
    /// Literal field not added to the dynamic table.
    LiteralWithoutIndexing,
    /// Literal field that intermediaries must never index.
    NeverIndexed,
    /// A representation written by a newer version of Crosswire.
    #[cfg_attr(feature = "serde", serde(other))]
    Unknown,
}

/// Per-field encoding hint, reserved for the frozen schema.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FieldHint {
    encoding: EncodingHint,
    huffman: bool,
}

impl FieldHint {
    /// Creates a hint.
    #[must_use]
    pub fn new(encoding: EncodingHint, huffman: bool) -> FieldHint {
        FieldHint { encoding, huffman }
    }

    /// The representation used.
    #[must_use]
    pub fn encoding(&self) -> EncodingHint {
        self.encoding
    }

    /// Whether Huffman coding was used.
    #[must_use]
    pub fn huffman(&self) -> bool {
        self.huffman
    }
}
