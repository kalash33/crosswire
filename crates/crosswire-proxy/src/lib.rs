//! HTTP/1.1 MITM proxy engine for Crosswire.

mod config;
mod counters;
mod error;
mod front;
pub(crate) mod generated;
mod handler;
mod info_page;
mod limits;
mod listener;
mod mitm;
mod record;
mod rejections;
mod relay;
mod sni;
mod tunnel;

pub use config::{ProxyAuth, ProxyConfig};
pub use counters::Counters;
pub use error::ProxyError;
pub use handler::{Decision, Handler, NoopHandler};
pub use limits::{ProxyLimits, ProxyLimitsBuilder};
#[doc(hidden)]
pub use listener::ResolveOverride;
pub use listener::{Proxy, ProxyDeps};
pub use sni::{parse_sni, SniError};
