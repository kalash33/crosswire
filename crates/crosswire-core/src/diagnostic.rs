//! The diagnostic catalogue.

use crate::{Profile, Span};

/// How serious a diagnostic is. The wire code is [`Severity::as_str`].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[non_exhaustive]
pub enum Severity {
    /// Informational.
    Info,
    /// Suspicious but accepted.
    Warning,
    /// Violates the specification.
    Error,
    /// A severity written by a newer version of Crosswire. It orders after
    /// [`Severity::Error`] only because it is declared last; it carries no
    /// meaning of its own.
    #[cfg_attr(feature = "serde", serde(other))]
    Unknown,
}

impl Severity {
    /// Stable snake_case name of the severity (its wire code).
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Warning => "warning",
            Severity::Error => "error",
            Severity::Unknown => "unknown",
        }
    }
}

/// Stable identifier of a kind of diagnostic. The wire code is
/// [`DiagnosticCode::as_str`] (the corpus `.expect.json` names).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[non_exhaustive]
pub enum DiagnosticCode {
    /// See [`DiagnosticCode::as_str`]: `duplicate_content_length`.
    DuplicateContentLength,
    /// See [`DiagnosticCode::as_str`]: `conflicting_framing`.
    ConflictingFraming,
    /// See [`DiagnosticCode::as_str`]: `obs_fold`.
    ObsFold,
    /// See [`DiagnosticCode::as_str`]: `bare_lf`.
    BareLf,
    /// See [`DiagnosticCode::as_str`]: `bare_cr`.
    BareCr,
    /// See [`DiagnosticCode::as_str`]: `whitespace_before_colon`.
    WhitespaceBeforeColon,
    /// See [`DiagnosticCode::as_str`]: `trailing_space_in_name`.
    TrailingSpaceInName,
    /// See [`DiagnosticCode::as_str`]: `leading_whitespace_before_start_line`.
    LeadingWhitespaceBeforeStartLine,
    /// See [`DiagnosticCode::as_str`]: `non_canonical_header_case`.
    NonCanonicalHeaderCase,
    /// See [`DiagnosticCode::as_str`]: `invalid_token_char_in_name`.
    InvalidTokenCharInName,
    /// See [`DiagnosticCode::as_str`]: `invalid_char_in_value`.
    InvalidCharInValue,
    /// See [`DiagnosticCode::as_str`]: `missing_host`.
    MissingHost,
    /// See [`DiagnosticCode::as_str`]: `duplicate_host`.
    DuplicateHost,
    /// See [`DiagnosticCode::as_str`]: `absolute_form_target`.
    AbsoluteFormTarget,
    /// See [`DiagnosticCode::as_str`]: `chunk_extension_present`.
    ChunkExtensionPresent,
    /// See [`DiagnosticCode::as_str`]: `trailer_present`.
    TrailerPresent,
    /// See [`DiagnosticCode::as_str`]: `chunk_size_leading_zeros`.
    ChunkSizeLeadingZeros,
    /// See [`DiagnosticCode::as_str`]: `invalid_chunk_size`.
    InvalidChunkSize,
    /// See [`DiagnosticCode::as_str`]: `content_length_non_numeric`.
    ContentLengthNonNumeric,
    /// See [`DiagnosticCode::as_str`]: `content_length_leading_plus`.
    ContentLengthLeadingPlus,
    /// See [`DiagnosticCode::as_str`]: `unknown_transfer_coding`.
    UnknownTransferCoding,
    /// See [`DiagnosticCode::as_str`]: `body_after_no_body_status`.
    BodyAfterNoBodyStatus,
    /// See [`DiagnosticCode::as_str`]: `expect_100_continue`.
    #[cfg_attr(feature = "serde", serde(rename = "expect_100_continue"))]
    Expect100Continue,
    /// See [`DiagnosticCode::as_str`]: `http_0_9_request`.
    #[cfg_attr(feature = "serde", serde(rename = "http_0_9_request"))]
    Http09Request,
    /// See [`DiagnosticCode::as_str`]: `version_unusual`.
    VersionUnusual,
    /// See [`DiagnosticCode::as_str`]: `chunk_size_whitespace`.
    ChunkSizeWhitespace,
    /// A code written by a newer version of Crosswire (`unknown`). Never
    /// raised by this parser and not part of [`DiagnosticCode::ALL`].
    #[cfg_attr(feature = "serde", serde(other))]
    Unknown,
}

impl DiagnosticCode {
    /// Every code this parser can raise, in declaration order
    /// ([`DiagnosticCode::Unknown`] excluded).
    pub const ALL: &'static [DiagnosticCode] = &[
        DiagnosticCode::DuplicateContentLength,
        DiagnosticCode::ConflictingFraming,
        DiagnosticCode::ObsFold,
        DiagnosticCode::BareLf,
        DiagnosticCode::BareCr,
        DiagnosticCode::WhitespaceBeforeColon,
        DiagnosticCode::TrailingSpaceInName,
        DiagnosticCode::LeadingWhitespaceBeforeStartLine,
        DiagnosticCode::NonCanonicalHeaderCase,
        DiagnosticCode::InvalidTokenCharInName,
        DiagnosticCode::InvalidCharInValue,
        DiagnosticCode::MissingHost,
        DiagnosticCode::DuplicateHost,
        DiagnosticCode::AbsoluteFormTarget,
        DiagnosticCode::ChunkExtensionPresent,
        DiagnosticCode::TrailerPresent,
        DiagnosticCode::ChunkSizeLeadingZeros,
        DiagnosticCode::InvalidChunkSize,
        DiagnosticCode::ContentLengthNonNumeric,
        DiagnosticCode::ContentLengthLeadingPlus,
        DiagnosticCode::UnknownTransferCoding,
        DiagnosticCode::BodyAfterNoBodyStatus,
        DiagnosticCode::Expect100Continue,
        DiagnosticCode::Http09Request,
        DiagnosticCode::VersionUnusual,
        DiagnosticCode::ChunkSizeWhitespace,
    ];

    /// Stable snake_case name of the code.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            DiagnosticCode::DuplicateContentLength => "duplicate_content_length",
            DiagnosticCode::ConflictingFraming => "conflicting_framing",
            DiagnosticCode::ObsFold => "obs_fold",
            DiagnosticCode::BareLf => "bare_lf",
            DiagnosticCode::BareCr => "bare_cr",
            DiagnosticCode::WhitespaceBeforeColon => "whitespace_before_colon",
            DiagnosticCode::TrailingSpaceInName => "trailing_space_in_name",
            DiagnosticCode::LeadingWhitespaceBeforeStartLine => {
                "leading_whitespace_before_start_line"
            }
            DiagnosticCode::NonCanonicalHeaderCase => "non_canonical_header_case",
            DiagnosticCode::InvalidTokenCharInName => "invalid_token_char_in_name",
            DiagnosticCode::InvalidCharInValue => "invalid_char_in_value",
            DiagnosticCode::MissingHost => "missing_host",
            DiagnosticCode::DuplicateHost => "duplicate_host",
            DiagnosticCode::AbsoluteFormTarget => "absolute_form_target",
            DiagnosticCode::ChunkExtensionPresent => "chunk_extension_present",
            DiagnosticCode::TrailerPresent => "trailer_present",
            DiagnosticCode::ChunkSizeLeadingZeros => "chunk_size_leading_zeros",
            DiagnosticCode::InvalidChunkSize => "invalid_chunk_size",
            DiagnosticCode::ContentLengthNonNumeric => "content_length_non_numeric",
            DiagnosticCode::ContentLengthLeadingPlus => "content_length_leading_plus",
            DiagnosticCode::UnknownTransferCoding => "unknown_transfer_coding",
            DiagnosticCode::BodyAfterNoBodyStatus => "body_after_no_body_status",
            DiagnosticCode::Expect100Continue => "expect_100_continue",
            DiagnosticCode::Http09Request => "http_0_9_request",
            DiagnosticCode::VersionUnusual => "version_unusual",
            DiagnosticCode::ChunkSizeWhitespace => "chunk_size_whitespace",
            DiagnosticCode::Unknown => "unknown",
        }
    }

    /// Default severity of this code under `profile`.
    #[must_use]
    pub fn default_severity(&self, profile: Profile) -> Severity {
        let (lenient, strict) = match self {
            DiagnosticCode::DuplicateContentLength => (Severity::Warning, Severity::Error),
            DiagnosticCode::ConflictingFraming => (Severity::Warning, Severity::Error),
            DiagnosticCode::ObsFold => (Severity::Warning, Severity::Error),
            DiagnosticCode::BareLf => (Severity::Warning, Severity::Error),
            DiagnosticCode::BareCr => (Severity::Warning, Severity::Error),
            DiagnosticCode::WhitespaceBeforeColon => (Severity::Warning, Severity::Error),
            DiagnosticCode::TrailingSpaceInName => (Severity::Warning, Severity::Error),
            DiagnosticCode::LeadingWhitespaceBeforeStartLine => (Severity::Info, Severity::Error),
            DiagnosticCode::NonCanonicalHeaderCase => (Severity::Info, Severity::Info),
            DiagnosticCode::InvalidTokenCharInName => (Severity::Warning, Severity::Error),
            DiagnosticCode::InvalidCharInValue => (Severity::Warning, Severity::Error),
            DiagnosticCode::MissingHost => (Severity::Warning, Severity::Error),
            DiagnosticCode::DuplicateHost => (Severity::Warning, Severity::Error),
            DiagnosticCode::AbsoluteFormTarget => (Severity::Info, Severity::Info),
            DiagnosticCode::ChunkExtensionPresent => (Severity::Info, Severity::Info),
            DiagnosticCode::TrailerPresent => (Severity::Info, Severity::Info),
            DiagnosticCode::ChunkSizeLeadingZeros => (Severity::Info, Severity::Warning),
            DiagnosticCode::InvalidChunkSize => (Severity::Error, Severity::Error),
            DiagnosticCode::ContentLengthNonNumeric => (Severity::Warning, Severity::Error),
            DiagnosticCode::ContentLengthLeadingPlus => (Severity::Warning, Severity::Error),
            DiagnosticCode::UnknownTransferCoding => (Severity::Warning, Severity::Error),
            DiagnosticCode::BodyAfterNoBodyStatus => (Severity::Warning, Severity::Error),
            DiagnosticCode::Expect100Continue => (Severity::Info, Severity::Info),
            DiagnosticCode::Http09Request => (Severity::Warning, Severity::Error),
            DiagnosticCode::VersionUnusual => (Severity::Warning, Severity::Error),
            DiagnosticCode::ChunkSizeWhitespace => (Severity::Info, Severity::Warning),
            DiagnosticCode::Unknown => (Severity::Unknown, Severity::Unknown),
        };
        match profile {
            Profile::Strict => strict,
            _ => lenient,
        }
    }
}

/// A finding about the input, located by a span.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Diagnostic {
    code: DiagnosticCode,
    span: Span,
    severity: Severity,
}

impl Diagnostic {
    /// Creates a diagnostic.
    #[must_use]
    pub fn new(code: DiagnosticCode, span: Span, severity: Severity) -> Diagnostic {
        Diagnostic {
            code,
            span,
            severity,
        }
    }

    /// The kind of finding.
    #[must_use]
    pub fn code(&self) -> DiagnosticCode {
        self.code
    }

    /// Where in the input it applies.
    #[must_use]
    pub fn span(&self) -> Span {
        self.span
    }

    /// Its severity.
    #[must_use]
    pub fn severity(&self) -> Severity {
        self.severity
    }
}
