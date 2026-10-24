//! Integration tests spanning the Crosswire recorder crates.
//!
//! The client-side recording recipe now lives in
//! [`crosswire_tools::record`]; this crate keeps a backward-compatible
//! four-argument [`record_exchange`] wrapper so the existing recorder tests
//! keep compiling. New code should call [`crosswire_tools::record_exchange`]
//! directly, which also files the exchange in a namespace, records a scope
//! verdict, attaches diffs and links a source exchange.

use std::sync::Arc;

use crosswire_client::{Session, WritePlan};
use crosswire_h1::ResponseContext;
use crosswire_model::{ExchangeSink, Namespace, ScopeState};
use ulid::Ulid;

pub use crosswire_tools::RecordError;

/// Sends `plan` over `session`, reads the whole response and submits the
/// exchange to `sink`; returns the exchange id.
///
/// This is the original four-argument recipe: it files the exchange in the
/// default (history) namespace with an empty scope verdict, no diffs and no
/// source link. See [`crosswire_tools::record_exchange`] for the full recipe.
///
/// # Errors
/// Propagates every [`RecordError`] from [`crosswire_tools::record_exchange`].
pub async fn record_exchange(
    session: &mut Session,
    plan: WritePlan,
    ctx: ResponseContext,
    sink: Arc<dyn ExchangeSink>,
) -> Result<Ulid, RecordError> {
    crosswire_tools::record_exchange(
        session,
        plan,
        ctx,
        sink,
        Namespace::default(),
        ScopeState::Empty,
        Vec::new(),
        None,
    )
    .await
    .map(|r| r.id)
}
