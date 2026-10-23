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
