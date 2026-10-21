//! Incremental parsing results.

use alloc::vec::Vec;

use crosswire_core::Span;

use crate::message::{HeaderSpan, RawMessage};

/// What a single parser step produced.
#[derive(Debug)]
#[non_exhaustive]
pub enum Progress {
    /// More input is needed.
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
    /// The message is complete.
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
