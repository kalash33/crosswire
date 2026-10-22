//! Errors returned by the upstream client.

use crate::gate::GateRefusal;

/// Why an upstream connection could not be made.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ClientError {
    /// Name resolution failed or found nothing.
    #[error("dns: {0}")]
    Dns(String),
    /// The destination gate refused the resolved address.
    #[error("destination refused: {0}")]
    Refused(GateRefusal),
    /// A socket error.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// The TLS handshake or certificate verification failed.
    #[error("tls: {0}")]
    Tls(String),
    /// A phase exceeded its time limit.
    #[error("timed out")]
    Timeout,
    /// The peer broke the protocol.
    #[error("protocol: {0}")]
    Protocol(String),
}
