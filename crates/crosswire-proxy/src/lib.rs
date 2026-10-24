//! HTTP/1.1 MITM proxy engine for Crosswire.

mod capture;
mod config;
mod counters;
mod error;
mod front;
pub(crate) mod generated;
mod handler;
mod info_page;
mod intercept;
mod limits;
mod listener;
mod mitm;
mod pump;
mod record;
mod rejections;
mod relay;
mod rewrite;
mod sni;
mod tunnel;

pub use capture::Budget;
pub use config::{ProxyAuth, ProxyConfig};
pub use counters::Counters;
pub use error::ProxyError;
pub use handler::{BoxFuture, Decision, Handler, HeldRequest, NoopHandler};
pub use intercept::{
    EditedBytes, HeldSummary, Intercept, InterceptError, InterceptRules, OnTimeout, Overflow,
};
pub use limits::{ProxyLimits, ProxyLimitsBuilder};
#[doc(hidden)]
pub use listener::ResolveOverride;
pub use listener::{Proxy, ProxyDeps};
pub use sni::{parse_sni, SniError};
