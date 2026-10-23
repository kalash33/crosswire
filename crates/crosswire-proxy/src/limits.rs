//! Resource limits for the proxy (spec section 11.6).

use std::time::Duration;

use crosswire_h1::ParserConfig;

const MIB: u64 = 1024 * 1024;

/// Resource limits applied by the proxy. Build with
/// [`ProxyLimits::builder`]; the defaults are the spec values.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ProxyLimits {
    /// Most bytes of one body kept in the capture.
    pub capture_cap: u64,
    /// Most bytes of one body held in memory before spilling.
    pub inline_memory: u64,
    /// Total in-flight body bytes across all connections.
    pub body_budget: u64,
    /// Time allowed to read a request head.
    pub header_read: Duration,
    /// Idle time allowed on a connection.
    pub idle: Duration,
    /// Total time allowed for one exchange.
    pub total: Duration,
    /// Time allowed for a TLS handshake.
    pub tls_handshake: Duration,
    /// Idle time allowed on an opaque tunnel (`None` = unlimited).
    pub tunnel_idle: Option<Duration>,
    /// Most simultaneous connections per listener.
    pub max_connections: usize,
    /// Most CONNECTs awaiting tunnel establishment.
    pub max_pending_connect: usize,
    /// Most connections per non-loopback client IP.
    pub max_per_client_ip: usize,
    /// Deepest pipelining queue.
    pub max_pipelined: usize,
    /// Handshake rejections per window that raise a marker.
    pub rejections_per_window: u32,
    /// The rejection-counting window.
    pub rejection_window: Duration,
    /// HTTP/1 parser configuration and limits.
    pub parse: ParserConfig,
}

impl Default for ProxyLimits {
    fn default() -> Self {
        ProxyLimits {
            capture_cap: 50 * MIB,
            inline_memory: MIB,
            body_budget: 256 * MIB,
            header_read: Duration::from_secs(30),
            idle: Duration::from_secs(60),
            total: Duration::from_secs(300),
            tls_handshake: Duration::from_secs(15),
            tunnel_idle: None,
            max_connections: 4_000,
            max_pending_connect: 256,
            max_per_client_ip: 64,
            max_pipelined: 16,
            rejections_per_window: 3,
            rejection_window: Duration::from_secs(30),
            parse: ParserConfig::default(),
        }
    }
}

impl ProxyLimits {
    /// Starts a builder at the default limits.
    #[must_use]
    pub fn builder() -> ProxyLimitsBuilder {
        ProxyLimitsBuilder {
            limits: ProxyLimits::default(),
        }
    }
}

/// Builder for [`ProxyLimits`].
#[derive(Clone, Debug)]
pub struct ProxyLimitsBuilder {
    limits: ProxyLimits,
}

macro_rules! setter {
    ($(#[$m:meta])* $name:ident: $ty:ty) => {
        $(#[$m])*
        #[must_use]
        pub fn $name(mut self, v: $ty) -> Self {
            self.limits.$name = v;
            self
        }
    };
}

impl ProxyLimitsBuilder {
    setter!(/// Sets `capture_cap`.
        capture_cap: u64);
    setter!(/// Sets `inline_memory`.
        inline_memory: u64);
    setter!(/// Sets `body_budget`.
        body_budget: u64);
    setter!(/// Sets `header_read`.
        header_read: Duration);
    setter!(/// Sets `idle`.
        idle: Duration);
    setter!(/// Sets `total`.
        total: Duration);
    setter!(/// Sets `tls_handshake`.
        tls_handshake: Duration);
    setter!(/// Sets `tunnel_idle`.
        tunnel_idle: Option<Duration>);
    setter!(/// Sets `max_connections`.
        max_connections: usize);
    setter!(/// Sets `max_pending_connect`.
        max_pending_connect: usize);
    setter!(/// Sets `max_per_client_ip`.
        max_per_client_ip: usize);
    setter!(/// Sets `max_pipelined`.
        max_pipelined: usize);
    setter!(/// Sets `rejections_per_window`.
        rejections_per_window: u32);
    setter!(/// Sets `rejection_window`.
        rejection_window: Duration);
    setter!(/// Sets `parse`.
        parse: ParserConfig);

    /// Finishes the builder.
    #[must_use]
    pub fn build(self) -> ProxyLimits {
        self.limits
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crosswire_h1::ParseLimits;

    #[test]
    fn defaults_match_spec() {
        let l = ProxyLimits::default();
        assert_eq!(l.capture_cap, 50 * 1024 * 1024);
        assert_eq!(l.inline_memory, 1024 * 1024);
        assert_eq!(l.body_budget, 256 * 1024 * 1024);
        assert_eq!(l.header_read, Duration::from_secs(30));
        assert_eq!(l.idle, Duration::from_secs(60));
        assert_eq!(l.total, Duration::from_secs(300));
        assert_eq!(l.tls_handshake, Duration::from_secs(15));
        assert_eq!(l.tunnel_idle, None);
        assert_eq!(l.max_connections, 4_000);
        assert_eq!(l.max_pending_connect, 256);
        assert_eq!(l.max_per_client_ip, 64);
        assert_eq!(l.max_pipelined, 16);
        assert_eq!(l.rejections_per_window, 3);
        assert_eq!(l.rejection_window, Duration::from_secs(30));
        let spec = ParseLimits::builder()
            .head_size(64 * 1024)
            .max_headers(256)
            .max_line(16 * 1024)
            .max_chunk_size_line(4 * 1024)
            .max_trailer_size(16 * 1024)
            .max_chunks(1_000_000)
            .max_body_diagnostics(64)
            .build();
        assert_eq!(*l.parse.limits(), spec);
        assert_eq!(l.parse, ParserConfig::default());
    }

    #[test]
    fn builder_overrides_one_field() {
        let l = ProxyLimits::builder().max_connections(2).build();
        assert_eq!(l.max_connections, 2);
        assert_eq!(l.max_pipelined, 16);
    }
}
