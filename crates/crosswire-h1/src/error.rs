//! Fatal parse errors.

use core::fmt;

use crosswire_core::Span;

/// A fatal error that stops parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParseError {
    /// The message head exceeded [`ParseLimits::head_size`](crate::ParseLimits::head_size).
    HeadTooLarge,
    /// A line exceeded the configured maximum length.
    LineTooLong,
    /// The head contained more headers than allowed.
    TooManyHeaders,
    /// Reserved; not currently returned. When no terminating blank line
    /// appears within [`ParseLimits::head_size`](crate::ParseLimits::head_size)
    /// the parser returns [`ParseError::HeadTooLarge`], and end of input
    /// before it is [`ParseError::UnexpectedEof`].
    NoTerminator,
    /// The input ended in the middle of a message.
    UnexpectedEof,
    /// A chunk-size line could not be parsed.
    InvalidChunkSize {
        /// Location of the offending chunk-size text.
        span: Span,
    },
    /// The trailer section exceeded its size limit.
    TrailerTooLarge,
    /// A chunked body had more data chunks than
    /// [`ParseLimits::max_chunks`](crate::ParseLimits::max_chunks).
    TooManyChunks,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::HeadTooLarge => f.write_str("message head too large"),
            ParseError::LineTooLong => f.write_str("line too long"),
            ParseError::TooManyHeaders => f.write_str("too many headers"),
            ParseError::NoTerminator => f.write_str("message head has no terminating blank line"),
            ParseError::UnexpectedEof => f.write_str("unexpected end of input"),
            ParseError::InvalidChunkSize { span } => write!(
                f,
                "invalid chunk size at bytes {}..{}",
                span.start(),
                span.end()
            ),
            ParseError::TrailerTooLarge => f.write_str("trailer section too large"),
            ParseError::TooManyChunks => f.write_str("too many chunks"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for ParseError {}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    #[test]
    fn display_each_variant() {
        let cases = [
            (ParseError::HeadTooLarge, "message head too large"),
            (ParseError::LineTooLong, "line too long"),
            (ParseError::TooManyHeaders, "too many headers"),
            (
                ParseError::NoTerminator,
                "message head has no terminating blank line",
            ),
            (ParseError::UnexpectedEof, "unexpected end of input"),
            (
                ParseError::InvalidChunkSize {
                    span: Span::new(3, 7),
                },
                "invalid chunk size at bytes 3..7",
            ),
            (ParseError::TrailerTooLarge, "trailer section too large"),
            (ParseError::TooManyChunks, "too many chunks"),
        ];
        for (err, msg) in cases {
            assert_eq!(err.to_string(), msg);
        }
    }
}
