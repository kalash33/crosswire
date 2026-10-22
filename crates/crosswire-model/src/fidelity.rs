//! Fidelity and kind labels.

use serde::{Deserialize, Serialize};

/// How faithfully a message reproduces what was on the wire.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum Fidelity {
    /// Exactly the bytes read from or written to upstream.
    Raw,
    /// Decoded but in wire order (HTTP/2 frames, v0.3).
    DecodedOrdered,
    /// Translated through an adapter or rewritten (for example redacted).
    Normalized,
    /// Counted but not parsed (tunnels).
    Opaque,
    /// Authored by the proxy itself (for example a 502 page).
    Generated,
}

/// The wire representation a [`Message`](crate::Message) head uses.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum MessageKind {
    /// HTTP/1 head bytes with spans.
    Http1Raw,
    /// HTTP/2 frames (v0.3; the head shape is added then).
    Http2Frames,
    /// A translated header list.
    Normalized,
    /// WebSocket frames (v0.3; the head shape is added then).
    WsFrames,
    /// An opaque tunnel with counters.
    Tunnel,
}
