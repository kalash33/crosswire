//! The hook interface the relay calls as requests and responses pass
//! through.
//!
//! The request path is asynchronous: the relay hands the parsed head (and,
//! when the body fit under `intercept_hold`, a buffered copy of it) to
//! [`Handler::on_request`] and awaits the [`Decision`]. The service behind
//! the handler may hold the request indefinitely (an operator intercept)
//! before resolving the future. The response-head hook stays synchronous:
//! response intercept is deferred.

use std::future::Future;
use std::pin::Pin;

use bytes::Bytes;
use crosswire_core::Change;
use crosswire_h1::RawMessage;
use crosswire_model::{ConnectionInfo, ScopeState};

/// A boxed future, so [`Handler`] stays object-safe. Mirrors
/// `crosswire_client::BoxFuture`.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// What the relay should do with a request.
///
/// `#[non_exhaustive]` so new variants are not a breaking change.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Decision {
    /// Pass it on unchanged (the buffered body, if any, is written first).
    Forward,
    /// Drop the exchange and close both connections.
    Drop,
    /// Forward edited bytes instead of the original: `head` and optional
    /// `body` go upstream, and `changes` are recorded on the request's
    /// diffs. Only produced for an editable (buffered) request.
    Modify {
        /// The request head to send upstream and record as canonical.
        head: Bytes,
        /// The request body to send upstream, if any.
        body: Option<Bytes>,
        /// The edits from the original request to the forwarded bytes.
        changes: Vec<Change>,
    },
    /// Answer the client with a proxy-generated 504 (a hold timed out).
    Respond504,
}

/// A request held for a handler decision.
///
/// Borrows the parsed head, the client connection facts, and — when the
/// body fit under `intercept_hold` and was buffered — the body bytes. The
/// future returned by [`Handler::on_request`] must not outlive these
/// borrows.
pub struct HeldRequest<'a> {
    /// The request head as the client sent it.
    pub head: &'a RawMessage,
    /// The request body, present only when it was buffered (fit under
    /// `intercept_hold` and the request is [`editable`](Self::editable)).
    pub body: Option<&'a Bytes>,
    /// The client-side connection facts.
    pub conn: &'a ConnectionInfo,
    /// The scope verdict for this exchange.
    pub scope_state: ScopeState,
    /// Whether the body was fully buffered, so the request may be edited.
    pub editable: bool,
}

/// Observes and steers traffic through the proxy.
pub trait Handler: Send + Sync {
    /// Whether this request will be held, decided from the head alone
    /// (cheap and synchronous). When it returns `true` the relay buffers
    /// the body (up to `intercept_hold`) so it can be edited before calling
    /// [`on_request`](Handler::on_request); when it returns `false` the body
    /// streams through unbuffered. The default never holds, so a forwarding
    /// handler keeps the relay's streaming behaviour.
    fn holds(&self, _head: &RawMessage, _conn: &ConnectionInfo, _scope_state: ScopeState) -> bool {
        false
    }
    /// Called once the request head (and, when `editable`, its body) is
    /// ready, before it is forwarded. May await an operator decision.
    fn on_request<'a>(&'a self, req: HeldRequest<'a>) -> BoxFuture<'a, Decision>;
    /// Called once the response head has been parsed, before it is
    /// forwarded. Synchronous: it must not block.
    fn on_response_head(&self, resp: &RawMessage, req: &RawMessage) -> Decision;
}

/// A [`Handler`] that forwards everything.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopHandler;

impl Handler for NoopHandler {
    fn on_request<'a>(&'a self, _req: HeldRequest<'a>) -> BoxFuture<'a, Decision> {
        Box::pin(async { Decision::Forward })
    }
    fn on_response_head(&self, _resp: &RawMessage, _req: &RawMessage) -> Decision {
        Decision::Forward
    }
}
