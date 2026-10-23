//! The proxy's error type.

use crosswire_client::ClientError;

/// Why a proxy operation failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ProxyError {
    /// The configuration is invalid.
    #[error("invalid proxy configuration: {0}")]
    Config(String),
    /// A non-loopback bind address was given without `allow_non_loopback`.
    #[error("refusing to bind a non-loopback address without --allow-non-loopback")]
    NonLoopbackBind,
    /// A non-loopback bind requires proxy authentication.
    #[error("proxy authentication is required when binding a non-loopback address")]
    AuthRequired,
    /// The listener could not bind.
    #[error("could not bind the proxy listener: {0}")]
    Bind(#[source] std::io::Error),
    /// An I/O error on a client or upstream connection.
    #[error("i/o error: {0}")]
    Io(#[source] std::io::Error),
    /// The upstream connection failed.
    #[error("upstream connection failed: {0}")]
    Client(#[from] ClientError),
    /// A TLS failure.
    #[error("tls error: {0}")]
    Tls(String),
    /// A protocol violation by a peer.
    #[error("protocol error: {0}")]
    Protocol(String),
    /// A phase exceeded its time limit.
    #[error("timed out: {0}")]
    Timeout(&'static str),
    /// A resource limit was exceeded.
    #[error("limit exceeded: {0}")]
    Limit(&'static str),
}
