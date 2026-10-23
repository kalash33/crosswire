//! HTTP/1.1 MITM proxy engine for Crosswire.

mod config;
mod error;
pub(crate) mod generated;
mod handler;
mod limits;

pub use config::{ProxyAuth, ProxyConfig};
pub use error::ProxyError;
pub use handler::{Decision, Handler, NoopHandler};
pub use limits::{ProxyLimits, ProxyLimitsBuilder};

/// Test-only access to crate internals; not part of the public API.
#[doc(hidden)]
pub mod __test {
    pub use crate::generated::{
        generated, BAD_GATEWAY_CONNECT, BAD_GATEWAY_TLS, GATEWAY_TIMEOUT, NOT_A_PROXY_REQUEST,
        REFUSED_DESTINATION,
    };
}
