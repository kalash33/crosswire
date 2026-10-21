//! Resource limits enforced while parsing.

/// Upper bounds on the sizes the parser will accept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ParseLimits {
    /// Maximum size in bytes of the whole message head.
    pub head_size: u32,
    /// Maximum number of header fields.
    pub max_headers: u16,
    /// Maximum length in bytes of a single line.
    pub max_line: u32,
    /// Maximum length in bytes of a chunk-size line (including extensions).
    pub max_chunk_size_line: u32,
    /// Maximum total size in bytes of the trailer section.
    pub max_trailer_size: u32,
}

impl Default for ParseLimits {
    fn default() -> Self {
        ParseLimits {
            head_size: 65536,
            max_headers: 256,
            max_line: 16384,
            max_chunk_size_line: 4096,
            max_trailer_size: 16384,
        }
    }
}

impl ParseLimits {
    /// Starts a builder initialised with the default limits.
    #[must_use]
    pub fn builder() -> ParseLimitsBuilder {
        ParseLimitsBuilder {
            limits: ParseLimits::default(),
        }
    }
}

/// Builder for [`ParseLimits`].
#[derive(Debug, Clone, Copy)]
pub struct ParseLimitsBuilder {
    limits: ParseLimits,
}

impl ParseLimitsBuilder {
    /// Sets [`ParseLimits::head_size`].
    #[must_use]
    pub fn head_size(mut self, v: u32) -> Self {
        self.limits.head_size = v;
        self
    }

    /// Sets [`ParseLimits::max_headers`].
    #[must_use]
    pub fn max_headers(mut self, v: u16) -> Self {
        self.limits.max_headers = v;
        self
    }

    /// Sets [`ParseLimits::max_line`].
    #[must_use]
    pub fn max_line(mut self, v: u32) -> Self {
        self.limits.max_line = v;
        self
    }

    /// Sets [`ParseLimits::max_chunk_size_line`].
    #[must_use]
    pub fn max_chunk_size_line(mut self, v: u32) -> Self {
        self.limits.max_chunk_size_line = v;
        self
    }

    /// Sets [`ParseLimits::max_trailer_size`].
    #[must_use]
    pub fn max_trailer_size(mut self, v: u32) -> Self {
        self.limits.max_trailer_size = v;
        self
    }

    /// Finishes the builder.
    #[must_use]
    pub fn build(self) -> ParseLimits {
        self.limits
    }
}
