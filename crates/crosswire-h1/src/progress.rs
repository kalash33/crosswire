//! Incremental parsing results.

use alloc::vec::Vec;

use crosswire_core::Span;

use crate::message::{HeaderSpan, RawMessage};

/// What a single parser step produced.
#[derive(Debug)]
#[non_exhaustive]
pub enum Progress {
    /// More input is needed.
    ///
    /// While reading the head, the parser buffers it: `consumed` covers all
    /// the input, and the caller feeds only new bytes next.
    ///
    /// In the body phase, tokens (a chunk-size line with its extension, and
    /// the trailer block) are never buffered: `consumed` stops at the start
    /// of the incomplete token (often 0), and the caller must present the
    /// unconsumed bytes again, followed by new data, on the next call. Chunk
    /// data is consumed incrementally.
    NeedMore,
    /// A complete message head.
    Head(RawMessage),
    /// Body bytes.
    BodyData {
        /// The raw bytes as framed on the wire.
        raw: Span,
        /// The decoded payload bytes.
        data: Span,
    },
    /// A chunk extension.
    ChunkExt(Span),
    /// The trailer fields.
    Trailers(Vec<HeaderSpan>),
    /// The message is complete. The parser keeps returning `Done`,
    /// consuming nothing, until [`Parser::reset`](crate::Parser::reset).
    Done,
}

/// The outcome of feeding input to the parser.
#[derive(Debug)]
pub struct Step {
    /// Bytes of input consumed.
    pub consumed: usize,
    /// What happened.
    pub event: Progress,
}
