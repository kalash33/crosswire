//! Event counters the proxy keeps for status output and tests.

use std::sync::atomic::AtomicU64;

/// Monotonic counters, updated with relaxed ordering.
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct Counters {
    /// Connections accepted (after the caps admitted them).
    pub accepted: AtomicU64,
    /// Connections closed at accept because `max_connections` was reached.
    pub refused_max_connections: AtomicU64,
    /// Connections closed at accept because their client IP was at
    /// `max_per_client_ip` (non-loopback peers only).
    pub refused_per_ip: AtomicU64,
    /// Connections closed at accept because `max_pending_connect` was
    /// reached.
    pub refused_pending_connect: AtomicU64,
    /// Requests refused with 407 for missing or wrong proxy credentials.
    pub auth_failures: AtomicU64,
    /// TLS handshakes the client aborted (an untrusted CA, usually).
    pub handshake_rejections: AtomicU64,
    /// Opaque tunnels opened.
    pub tunnels: AtomicU64,
    /// Exchanges relayed.
    pub exchanges: AtomicU64,
    /// Exchanges or markers the sink refused because it was full.
    pub sink_full: AtomicU64,
}
