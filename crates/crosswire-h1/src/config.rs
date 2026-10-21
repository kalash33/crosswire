//! Parser configuration.

use crosswire_core::Profile;

use crate::limits::ParseLimits;

/// Configuration for a parser: strictness profile and size limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct ParserConfig {
    profile: Profile,
    limits: ParseLimits,
}

impl ParserConfig {
    /// Starts a builder initialised with the defaults (lenient profile,
    /// default limits).
    #[must_use]
    pub fn builder() -> ParserConfigBuilder {
        ParserConfigBuilder {
            config: ParserConfig::default(),
        }
    }

    /// The strictness profile.
    #[must_use]
    pub fn profile(&self) -> Profile {
        self.profile
    }

    /// The size limits.
    #[must_use]
    pub fn limits(&self) -> &ParseLimits {
        &self.limits
    }
}

/// Builder for [`ParserConfig`].
#[derive(Debug, Clone, Copy)]
pub struct ParserConfigBuilder {
    config: ParserConfig,
}

impl ParserConfigBuilder {
    /// Sets the strictness profile.
    #[must_use]
    pub fn profile(mut self, profile: Profile) -> Self {
        self.config.profile = profile;
        self
    }

    /// Sets the size limits.
    #[must_use]
    pub fn limits(mut self, limits: ParseLimits) -> Self {
        self.config.limits = limits;
        self
    }

    /// Finishes the builder.
    #[must_use]
    pub fn build(self) -> ParserConfig {
        self.config
    }
}
