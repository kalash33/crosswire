//! Gated TCP and TLS connections to upstream targets.

use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crosswire_model::{ChainResult, ConnectionInfo};
use rustls_pki_types::ServerName;
use tokio::net::TcpStream;
use tokio::sync::Semaphore;

use crate::error::ClientError;
use crate::gate::DestinationGate;
use crate::tls::{Conn, TlsBackend, TlsConnectConfig, VerifyMode};

/// Process-wide connection id counter.
static NEXT_CONN_ID: AtomicU64 = AtomicU64::new(1);

const DNS_TIMEOUT: Duration = Duration::from_secs(5);
const DNS_CONCURRENCY: usize = 64;
const TCP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Where to connect, and how.
#[derive(Clone, Debug)]
pub struct Target {
    host: String,
    port: u16,
    tls: Option<TlsConnectConfig>,
    /// Set when the requested SNI is not a valid server name; connecting then
    /// fails rather than verifying against some other name.
    sni_error: Option<String>,
}

impl Target {
    /// A plain TCP target.
    #[must_use]
    pub fn plain(host: &str, port: u16) -> Target {
        Target {
            host: host.to_string(),
            port,
            tls: None,
            sni_error: None,
        }
    }

    /// A TLS target; `sni` is both sent and verified against.
    #[must_use]
    pub fn tls(host: &str, port: u16, sni: &str, verify: VerifyMode) -> Target {
        let mut t = Target::plain(host, port);
        let mut cfg = TlsConnectConfig::new(verify);
        match ServerName::try_from(sni.to_string()) {
            Ok(name) => cfg = cfg.with_sni(name),
            Err(_) => t.sni_error = Some(format!("invalid server name {sni:?}")),
        }
        t.tls = Some(cfg);
        t
    }

    /// A TLS target with a caller-built configuration. A config without an SNI
    /// uses `host` as the server name.
    #[must_use]
    pub fn with_tls_config(host: &str, port: u16, cfg: TlsConnectConfig) -> Target {
        let mut t = Target::plain(host, port);
        t.tls = Some(cfg);
        t
    }

    /// The host as requested.
    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }

    /// The port.
    #[must_use]
    pub fn port(&self) -> u16 {
        self.port
    }
}

/// An established upstream connection with what was recorded about it.
pub struct Connection {
    io: Box<dyn Conn>,
    info: ConnectionInfo,
    connect_ms: u32,
    tls_ms: u32,
    chain_error: Option<String>,
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection")
            .field("info", &self.info)
            .field("connect_ms", &self.connect_ms)
            .field("tls_ms", &self.tls_ms)
            .finish_non_exhaustive()
    }
}

impl Connection {
    /// The recorded connection facts.
    #[must_use]
    pub fn info(&self) -> &ConnectionInfo {
        &self.info
    }

    /// Milliseconds spent on TCP connect.
    #[must_use]
    pub fn connect_ms(&self) -> u32 {
        self.connect_ms
    }

    /// Milliseconds spent on the TLS handshake (0 for plain TCP).
    #[must_use]
    pub fn tls_ms(&self) -> u32 {
        self.tls_ms
    }

    /// Why the chain failed when it was overridden.
    #[must_use]
    pub fn chain_error(&self) -> Option<&str> {
        self.chain_error.as_deref()
    }

    /// Mutable access to the byte stream.
    pub fn io_mut(&mut self) -> &mut Box<dyn Conn> {
        &mut self.io
    }

    /// Splits into stream, facts, `connect_ms` and `tls_ms`.
    #[must_use]
    pub fn into_parts(self) -> (Box<dyn Conn>, ConnectionInfo, u32, u32) {
        (self.io, self.info, self.connect_ms, self.tls_ms)
    }
}

fn ms(since: Instant) -> u32 {
    u32::try_from(since.elapsed().as_millis()).unwrap_or(u32::MAX)
}

/// Connects to targets: DNS, the destination gate, TCP, then TLS.
pub struct Transport {
    backend: Arc<dyn TlsBackend>,
    gate: Arc<DestinationGate>,
    dns_timeout: Duration,
    dns_semaphore: Arc<Semaphore>,
}

impl std::fmt::Debug for Transport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Transport").finish_non_exhaustive()
    }
}

impl Transport {
    /// Creates a transport.
    #[must_use]
    pub fn new(backend: Arc<dyn TlsBackend>, gate: Arc<DestinationGate>) -> Transport {
        Transport {
            backend,
            gate,
            dns_timeout: DNS_TIMEOUT,
            dns_semaphore: Arc::new(Semaphore::new(DNS_CONCURRENCY)),
        }
    }

    async fn resolve(&self, host: &str, port: u16) -> Result<Vec<IpAddr>, ClientError> {
        if host.is_empty() {
            return Err(ClientError::Dns("empty host".into()));
        }
        let _permit = self
            .dns_semaphore
            .acquire()
            .await
            .map_err(|_| ClientError::Dns("resolver unavailable".into()))?;
        let lookup = tokio::net::lookup_host((host, port));
        let addrs = tokio::time::timeout(self.dns_timeout, lookup)
            .await
            .map_err(|_| ClientError::Dns("lookup timed out".into()))?
            .map_err(|e| ClientError::Dns(e.to_string()))?;
        let mut ips: Vec<IpAddr> = Vec::new();
        for a in addrs {
            if !ips.contains(&a.ip()) {
                ips.push(a.ip());
            }
        }
        if ips.is_empty() {
            return Err(ClientError::Dns(format!("no addresses for {host}")));
        }
        Ok(ips)
    }

    /// Resolves, gates, connects and (for TLS targets) handshakes.
    ///
    /// The gate sees the resolved addresses, and the connection goes to those
    /// same addresses, so the name is not resolved a second time.
    ///
    /// # Errors
    /// [`ClientError::Refused`] when the gate refuses any resolved address;
    /// otherwise DNS, I/O, TLS or timeout errors.
    pub async fn connect(&self, target: &Target) -> Result<Connection, ClientError> {
        let host = target.host.trim_start_matches('[').trim_end_matches(']');
        let ips = self.resolve(host, target.port).await?;
        self.gate
            .check(host, target.port, &ips)
            .map_err(ClientError::Refused)?;

        let t0 = Instant::now();
        let (tcp, ip) = connect_tcp(&ips, target.port).await?;
        let connect_ms = ms(t0);
        let _ = tcp.set_nodelay(true);

        // The configured host is the target (spec 4.6); the resolved
        // address actually dialled is the peer.
        let mut info = ConnectionInfo::new(target.host(), target.port)
            .with_peer_addr(ip.to_string())
            .with_conn(NEXT_CONN_ID.fetch_add(1, Ordering::Relaxed), false);

        let Some(cfg) = target.tls.clone() else {
            return Ok(Connection {
                io: Box::new(tcp),
                info: info.with_chain_result(ChainResult::NotTls),
                connect_ms,
                tls_ms: 0,
                chain_error: None,
            });
        };
        if let Some(e) = &target.sni_error {
            return Err(ClientError::Tls(e.clone()));
        }
        let mut cfg = cfg;
        if cfg.sni.is_none() {
            let name = ServerName::try_from(host.to_string())
                .map_err(|_| ClientError::Tls(format!("invalid server name {host:?}")))?;
            cfg = cfg.with_sni(name);
        }
        if let Some(name) = &cfg.sni {
            info = info.with_sni(name.to_str().into_owned());
        }

        let t1 = Instant::now();
        let (io, tls) = self.backend.connect(tcp, cfg).await?;
        let tls_ms = ms(t1);
        info = info
            .with_tls(tls.version.clone(), tls.cipher.clone())
            .with_chain_result(tls.chain_result.clone())
            .with_cert_override(tls.chain_error.is_some());
        if let Some(a) = &tls.alpn {
            info = info.with_alpn(a.clone());
        }
        Ok(Connection {
            io,
            info,
            connect_ms,
            tls_ms,
            chain_error: tls.chain_error,
        })
    }
}

async fn connect_tcp(ips: &[IpAddr], port: u16) -> Result<(TcpStream, IpAddr), ClientError> {
    let mut last: Option<ClientError> = None;
    for &ip in ips {
        match tokio::time::timeout(
            TCP_CONNECT_TIMEOUT,
            TcpStream::connect(SocketAddr::new(ip, port)),
        )
        .await
        {
            Ok(Ok(s)) => return Ok((s, ip)),
            Ok(Err(e)) => last = Some(ClientError::Io(e)),
            Err(_) => last = Some(ClientError::Timeout),
        }
    }
    Err(last.unwrap_or(ClientError::Timeout))
}
