//! The hook interface the relay calls as messages pass through.

use crosswire_h1::RawMessage;
use crosswire_model::ConnectionInfo;

/// What the relay should do with a message or chunk.
///
/// `Hold` and `Modify` arrive with intercept in alpha.2; the enum is
/// `#[non_exhaustive]` so adding them is not a breaking change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Decision {
    /// Pass it on unchanged.
    Forward,
    /// Drop the exchange and close both connections.
    Drop,
}

/// Observes and steers traffic through the proxy.
///
/// Methods are synchronous and must not block: the relay calls them inline.
pub trait Handler: Send + Sync {
    /// Called once the request head has been parsed, before it is forwarded.
    fn on_request_head(&self, req: &RawMessage, conn: &ConnectionInfo) -> Decision;
    /// Called for each request body chunk (decoded data).
    fn on_request_body_chunk(&self, data: &[u8]) -> Decision;
    /// Called once the response head has been parsed, before it is forwarded.
    fn on_response_head(&self, resp: &RawMessage, req: &RawMessage) -> Decision;
    /// Called for each response body chunk (decoded data).
    fn on_response_body_chunk(&self, data: &[u8]) -> Decision;
}

/// A [`Handler`] that forwards everything.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopHandler;

impl Handler for NoopHandler {
    fn on_request_head(&self, _req: &RawMessage, _conn: &ConnectionInfo) -> Decision {
        Decision::Forward
    }
    fn on_request_body_chunk(&self, _data: &[u8]) -> Decision {
        Decision::Forward
    }
    fn on_response_head(&self, _resp: &RawMessage, _req: &RawMessage) -> Decision {
        Decision::Forward
    }
    fn on_response_body_chunk(&self, _data: &[u8]) -> Decision {
        Decision::Forward
    }
}
