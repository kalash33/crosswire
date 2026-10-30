//! The frozen Crosswire exchange schema, events and sink traits.
//!
//! This crate is the single definition of what the store writes, the API
//! serves and the proxy fills (design spec section 6.1). From the 0.1.0 tag
//! the schema is **additive-only**: fields and variants may be added, never
//! removed or renamed, and readers ignore unknown fields. For that reason no
//! type here uses `deny_unknown_fields`, every enum is `#[non_exhaustive]`
//! with kebab-case wire names, and fields that a later version may add have
//! `#[serde(default)]`. [`SCHEMA_VERSION`] travels in the API envelope.
//!
//! Additive-only applies to fields and to enum variants: a newer writer may
//! add either, and an older reader never fails on them. Every enum on the
//! wire has an `Unknown` variant that an unrecognised code reads as: the
//! plain string enums ([`Fidelity`], [`MessageKind`], [`MarkerKind`],
//! [`TunnelReason`], [`ExchangeState`], [`ScopeState`], [`NamespaceKind`],
//! and `crosswire-core`'s `ChangeOp`, `ChangeReason`, `EncodingHint`,
//! `Severity`, `DiagnosticCode`) on an unknown string, and the enums tagged
//! by a `type` key ([`Head`], [`Body`], [`ChainResult`], [`Event`],
//! [`StartLineSpans`], [`FramingRecord`]) on an unknown `type`. Codes are
//! kebab-case, except `Severity` and `DiagnosticCode`, whose snake_case wire
//! codes equal their `as_str()` names (the corpus `.expect.json` names).
//!
//! Byte payloads serialise as base64 strings under the default `std`
//! feature; see the `bytes_serde` module notes in the source for the full
//! wire choice. [`Exchange::new`] and [`Marker::new`] need `std` (they read
//! the clock and a random source for the ULID); use [`Exchange::with_id`]
//! and [`Marker::with_id`] without it.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

#[cfg(feature = "std")]
mod b64;
mod bytes_serde;
mod event;
mod exchange;
mod fidelity;
mod marker;
mod message;
mod redact;
mod sink;
mod ulid_serde;

pub use event::Event;
pub use exchange::{Exchange, ExchangeState, Namespace, NamespaceKind, ScopeState, Timing};
pub use fidelity::{Fidelity, MessageKind};
pub use marker::{Marker, MarkerKind};
pub use message::{
    Body, Capture, ChainResult, ConnectionInfo, Diff, FramingRecord, Head, HeadSpans,
    HeaderSpanRecord, Message, StartLineSpans, TunnelReason, HTTP1_PARSER_VERSION,
};
pub use redact::RedactionPolicy;
pub use sink::{ExchangeSink, SinkError};

/// Version of the stored and served schema. Bumped only by a change that
/// older readers cannot ignore (which the additive-only rule forbids after
/// 0.1.0, so in practice it moves only on a deliberate break). 2 is the
/// 0.1.0 freeze: stable codes with `Unknown` fallbacks, `type`-tagged
/// variant families, and no stored `cert_override`.
pub const SCHEMA_VERSION: u32 = 2;
