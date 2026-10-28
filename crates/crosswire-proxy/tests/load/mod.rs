//! In-process HTTP/1.1 load harness shared by the §13 performance gates.
//!
//! It provides three pieces the gate binaries compose:
//!
//! * [`spawn_echo_upstream`] — a keep-alive HTTP/1.1 origin that returns a
//!   fixed-size body for every request.
//! * [`drive`] — opens `conns` keep-alive TCP connections that each issue
//!   requests back-to-back for a fixed duration, either straight to the
//!   upstream ([`Target::Direct`]) or through a running proxy in absolute
//!   form ([`Target::ViaProxy`]), recording per-request wall-clock latency.
//! * [`Stats`] / [`percentiles`] — a nearest-rank p50/p99 reducer over the
//!   merged per-connection latency samples, plus total requests and
//!   requests-per-second.
//!
//! The harness never starts a proxy itself: a gate task spawns one with
//! [`crate::common::ProxyHarness`] and passes its address to
//! [`Target::ViaProxy`]. This module is pulled into each gate binary with
//! `#[path = "load/mod.rs"] mod load;`, so not every item is used by every
//! binary; `dead_code` is allowed accordingly.
#![allow(dead_code)]

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crosswire_ca::{Ca, IssuerLimits, LeafIssuer};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use rustls::{ClientConfig, RootCertStore, ServerConfig};
use rustls_pki_types::{CertificateDer, ServerName};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::client::TlsStream;
use tokio_rustls::{TlsAcceptor, TlsConnector};

/// Merged latency and throughput for one [`drive`] run.
#[derive(Clone, Copy, Debug)]
pub struct Stats {
    /// Median per-request latency, microseconds (nearest-rank).
    pub p50_us: u64,
    /// 99th-percentile per-request latency, microseconds (nearest-rank).
    pub p99_us: u64,
    /// Total requests that completed across every connection.
    pub reqs: u64,
    /// Wall-clock time the drive actually ran.
    pub elapsed: Duration,
}

impl Stats {
    /// Completed requests per second over [`Stats::elapsed`]; `0.0` when no
    /// time elapsed.
    pub fn reqs_per_sec(&self) -> f64 {
        let secs = self.elapsed.as_secs_f64();
        if secs > 0.0 {
            self.reqs as f64 / secs
        } else {
            0.0
        }
    }
}

/// Where [`drive`] sends its requests.
pub enum Target {
    /// Connect straight to this upstream address (origin-form requests).
    Direct(SocketAddr),
    /// Connect to a running proxy and send absolute-form requests for
    /// `upstream_authority` (e.g. `"u.test:8080"`).
    ViaProxy {
        /// The proxy's listening address.
        proxy: SocketAddr,
        /// The upstream authority the proxy should relay to.
        upstream_authority: String,
    },
}

/// Nearest-rank p50 and p99 (microseconds) over `samples`, which is sorted
/// in place. Returns `(0, 0)` for an empty set.
pub fn percentiles(samples: &mut [u64]) -> (u64, u64) {
    if samples.is_empty() {
        return (0, 0);
    }
    samples.sort_unstable();
    (nearest_rank(samples, 0.50), nearest_rank(samples, 0.99))
}

/// The nearest-rank value at `q` (0.0..=1.0) of an already-sorted,
/// non-empty slice.
fn nearest_rank(sorted: &[u64], q: f64) -> u64 {
    let n = sorted.len();
    // rank = ceil(q * n), clamped to 1..=n, then 0-based.
    let rank = (q * n as f64).ceil() as usize;
    let idx = rank.clamp(1, n) - 1;
    sorted[idx]
}

/// Spawns a keep-alive HTTP/1.1 upstream on an ephemeral loopback port that
/// answers every request with a `body_len`-byte body (`Content-Length`
/// set). Each request is read fully before the response is written.
/// Returns the bound address; the server runs until the process exits.
pub async fn spawn_echo_upstream(body_len: usize) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    // Pre-render the full response once; every reply is a copy of it.
    let response: Arc<[u8]> = {
        let mut r = format!("HTTP/1.1 200 OK\r\nContent-Length: {body_len}\r\n\r\n").into_bytes();
        r.resize(r.len() + body_len, b'a');
        r.into()
    };
    tokio::spawn(async move {
        loop {
            let Ok((conn, _)) = listener.accept().await else {
                break;
            };
            let _ = conn.set_nodelay(true);
            tokio::spawn(serve_echo(conn, Arc::clone(&response)));
        }
    });
    addr
}

/// A TLS echo upstream for the MITM added-latency gate, plus the DER of the
/// fresh CA its per-SNI leaves chain to (hand it to the proxy's
/// `UpstreamWiring::loopback`, and to the direct baseline's client as its
/// only trust root).
pub struct TlsEchoUpstream {
    /// Where it listens (loopback, ephemeral port).
    pub addr: SocketAddr,
    /// The upstream CA certificate, DER.
    pub ca_der: Vec<u8>,
}

/// Issues a leaf for whatever SNI the client sends (an IP SAN for 127.0.0.1
/// when none is sent), signed by the upstream's own CA.
struct PerSni(LeafIssuer);

impl std::fmt::Debug for PerSni {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PerSni")
    }
}

impl ResolvesServerCert for PerSni {
    fn resolve(&self, hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        let name = match hello.server_name() {
            Some(n) => ServerName::try_from(n.to_string()).ok()?,
            None => ServerName::IpAddress(IpAddr::V4(Ipv4Addr::LOCALHOST).into()),
        };
        self.0.issue(&name).ok()
    }
}

/// A TLS acceptor backed by a fresh CA issuing per-SNI leaves (ring provider,
/// `http/1.1` ALPN).
fn upstream_tls_acceptor() -> (TlsAcceptor, Arc<Ca>) {
    let ca = Arc::new(Ca::generate("perf-upstream").unwrap());
    let issuer = LeafIssuer::new(Arc::clone(&ca), IssuerLimits::default());
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut cfg = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_cert_resolver(Arc::new(PerSni(issuer)));
    cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
    (TlsAcceptor::from(Arc::new(cfg)), ca)
}

/// Spawns a keep-alive HTTPS/1.1 upstream on an ephemeral loopback port: the
/// TLS sibling of [`spawn_echo_upstream`]. Every request is answered with a
/// fixed `body_len`-byte body (`Content-Length` set); the handshake uses a
/// per-SNI leaf from the returned CA. Runs until the process exits.
pub async fn spawn_tls_echo_upstream(body_len: usize) -> TlsEchoUpstream {
    let (acceptor, ca) = upstream_tls_acceptor();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let response: Arc<[u8]> = {
        let mut r = format!("HTTP/1.1 200 OK\r\nContent-Length: {body_len}\r\n\r\n").into_bytes();
        r.resize(r.len() + body_len, b'a');
        r.into()
    };
    tokio::spawn(async move {
        loop {
            let Ok((conn, _)) = listener.accept().await else {
                break;
            };
            let _ = conn.set_nodelay(true);
            let acceptor = acceptor.clone();
            let response = Arc::clone(&response);
            tokio::spawn(async move {
                let Ok(tls) = acceptor.accept(conn).await else {
                    return;
                };
                serve_echo(tls, response).await;
            });
        }
    });
    TlsEchoUpstream {
        addr,
        ca_der: ca.cert_der(),
    }
}

/// Serves one keep-alive connection: read each complete request head (and
/// any `Content-Length` body) then write the fixed response, until the peer
/// closes. Requests here are GETs (no body), but a `Content-Length` body is
/// drained so the loop frames back-to-back pipelined requests correctly.
async fn serve_echo<S: AsyncRead + AsyncWrite + Unpin>(mut conn: S, response: Arc<[u8]>) {
    let mut carry: Vec<u8> = Vec::with_capacity(16 * 1024);
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        // Ensure a full head (through the blank line) is buffered.
        let head_end = loop {
            if let Some(pos) = find_subslice(&carry, b"\r\n\r\n") {
                break pos + 4;
            }
            match conn.read(&mut buf).await {
                Ok(0) | Err(_) => return,
                Ok(n) => carry.extend_from_slice(&buf[..n]),
            }
        };
        // Drain a declared request body, if any, so framing stays aligned.
        let body_len = content_length(&carry[..head_end]).unwrap_or(0);
        let total = head_end + body_len;
        while carry.len() < total {
            match conn.read(&mut buf).await {
                Ok(0) | Err(_) => return,
                Ok(n) => carry.extend_from_slice(&buf[..n]),
            }
        }
        carry.drain(..total);
        if conn.write_all(&response).await.is_err() {
            return;
        }
    }
}

/// Parses a `Content-Length` value from a request head, if present.
fn content_length(head: &[u8]) -> Option<usize> {
    let text = std::str::from_utf8(head).ok()?;
    for line in text.split("\r\n") {
        let (name, value) = line.split_once(':')?;
        if name.eq_ignore_ascii_case("content-length") {
            return value.trim().parse().ok();
        }
    }
    None
}

/// Finds the first occurrence of `needle` in `haystack`.
fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Opens `conns` keep-alive connections to `target`, each issuing requests
/// back-to-back for `duration`, and returns the merged latency percentiles,
/// total requests, and elapsed time.
///
/// `body_len` is the size of the upstream **response** body being relayed;
/// the requests themselves are bodyless GETs. For [`Target::Direct`] the
/// connection goes straight to the upstream in origin form; for
/// [`Target::ViaProxy`] it goes to the proxy in absolute form.
pub async fn drive(target: Target, conns: usize, duration: Duration, body_len: usize) -> Stats {
    // Resolve the connect address and the request line each connection sends.
    let (connect_addr, request) = match &target {
        Target::Direct(addr) => {
            let req = b"GET / HTTP/1.1\r\nHost: upstream.load\r\n\r\n".to_vec();
            (*addr, req)
        }
        Target::ViaProxy {
            proxy,
            upstream_authority,
        } => {
            // Absolute-form request: the proxy relays it to the authority.
            let host = upstream_authority
                .rsplit_once(':')
                .map_or(upstream_authority.as_str(), |(h, _)| h);
            let req = format!("GET http://{upstream_authority}/ HTTP/1.1\r\nHost: {host}\r\n\r\n")
                .into_bytes();
            (*proxy, req)
        }
    };
    let request: Arc<[u8]> = request.into();

    // The expected response: a 200 with exactly `body_len` body bytes.
    let want_body = body_len;

    let start = Instant::now();
    let deadline = start + duration;

    let mut handles = Vec::with_capacity(conns);
    for _ in 0..conns {
        let request = Arc::clone(&request);
        handles.push(tokio::spawn(async move {
            drive_one(connect_addr, &request, want_body, deadline).await
        }));
    }

    let mut all: Vec<u64> = Vec::new();
    for h in handles {
        if let Ok(samples) = h.await {
            all.extend(samples);
        }
    }
    let elapsed = start.elapsed();

    let reqs = all.len() as u64;
    let (p50_us, p99_us) = percentiles(&mut all);
    Stats {
        p50_us,
        p99_us,
        reqs,
        elapsed,
    }
}

/// Runs one keep-alive connection until `deadline`, returning the per-request
/// latencies (microseconds). A connection error ends that connection's loop
/// and returns whatever it had measured.
async fn drive_one(
    addr: SocketAddr,
    request: &[u8],
    want_body: usize,
    deadline: Instant,
) -> Vec<u64> {
    let Ok(conn) = TcpStream::connect(addr).await else {
        return Vec::new();
    };
    let _ = conn.set_nodelay(true);
    drive_one_stream(conn, request, want_body, 0, deadline).await
}

/// Runs `warmup` unmeasured requests, then the measured request loop, over an
/// already-established stream (plain TCP, or a TLS stream whose handshake and
/// any `CONNECT` are already done). The warmup excludes a proxy's lazily-made
/// upstream connection (and cache warmup) from the per-request samples, so the
/// gate sees steady-state latency on an established connection; it is applied
/// equally to both measured paths. Returns the per-request latencies
/// (microseconds) until `deadline` or the first error.
async fn drive_one_stream<S: AsyncRead + AsyncWrite + Unpin>(
    mut conn: S,
    request: &[u8],
    want_body: usize,
    warmup: usize,
    deadline: Instant,
) -> Vec<u64> {
    let mut latencies: Vec<u64> = Vec::new();
    let mut carry: Vec<u8> = Vec::with_capacity(want_body + 256);
    let mut buf = vec![0u8; 64 * 1024];

    // Unmeasured warmup round-trips: force the proxy's upstream connection and
    // warm every cache before the first sample is taken.
    for _ in 0..warmup {
        if Instant::now() >= deadline {
            return latencies;
        }
        if conn.write_all(request).await.is_err() {
            return latencies;
        }
        if read_one_response(&mut conn, &mut carry, &mut buf, want_body)
            .await
            .is_none()
        {
            return latencies;
        }
    }

    while Instant::now() < deadline {
        let sent = Instant::now();
        if conn.write_all(request).await.is_err() {
            break;
        }
        // Read one full response: head through blank line, then `want_body`.
        if read_one_response(&mut conn, &mut carry, &mut buf, want_body)
            .await
            .is_none()
        {
            break;
        }
        latencies.push(sent.elapsed().as_micros() as u64);
    }
    latencies
}

/// Consumes exactly one HTTP/1.1 response (head + `want_body` body bytes)
/// from `conn`, buffering leftovers in `carry`. Returns `Some(())` on a
/// framed response, or `None` on EOF/error.
async fn read_one_response<S: AsyncRead + Unpin>(
    conn: &mut S,
    carry: &mut Vec<u8>,
    buf: &mut [u8],
    want_body: usize,
) -> Option<()> {
    let head_end = loop {
        if let Some(pos) = find_subslice(carry, b"\r\n\r\n") {
            break pos + 4;
        }
        match conn.read(buf).await {
            Ok(0) | Err(_) => return None,
            Ok(n) => carry.extend_from_slice(&buf[..n]),
        }
    };
    // The echo upstream always frames with Content-Length; trust the wanted
    // size, but fall back to any declared length so relayed responses frame.
    let body_len = content_length(&carry[..head_end]).unwrap_or(want_body);
    let total = head_end + body_len;
    while carry.len() < total {
        match conn.read(buf).await {
            Ok(0) | Err(_) => return None,
            Ok(n) => carry.extend_from_slice(&buf[..n]),
        }
    }
    carry.drain(..total);
    Some(())
}

/// Where [`drive_mitm`] sends its client-TLS requests. Both arms speak
/// origin-form HTTP/1.1 over a client TLS session to the same kind of
/// upstream; they differ only by the proxy hop, so a `Direct`/`ViaMitm` pair
/// isolates the proxy's added latency.
pub enum TlsTarget {
    /// Client TLS straight to the upstream at `addr`, trusting `ca_der`, SNI
    /// `server_name`.
    Direct {
        /// The upstream's listening address.
        addr: SocketAddr,
        /// The upstream CA to trust (DER).
        ca_der: Vec<u8>,
        /// The SNI/`Host` the client presents.
        server_name: String,
    },
    /// Through a MITM proxy: TCP to `proxy`, a `CONNECT authority` tunnel,
    /// then client TLS trusting the proxy's `ca_der` (it mints the leaf), SNI
    /// `server_name`.
    ViaMitm {
        /// The proxy's listening address.
        proxy: SocketAddr,
        /// The `CONNECT` target, e.g. `"u.test:8443"`.
        authority: String,
        /// The proxy CA to trust (DER).
        ca_der: Vec<u8>,
        /// The SNI/`Host` the client presents.
        server_name: String,
    },
}

impl TlsTarget {
    fn server_name(&self) -> &str {
        match self {
            TlsTarget::Direct { server_name, .. } | TlsTarget::ViaMitm { server_name, .. } => {
                server_name
            }
        }
    }
}

/// The TLS sibling of [`drive`]: opens `conns` keep-alive client-TLS sessions
/// to `target`, each issuing origin-form `GET /` requests back-to-back for
/// `duration`, and returns the merged per-request latency percentiles. The
/// per-connection TLS handshake (and, for [`TlsTarget::ViaMitm`], the
/// `CONNECT` setup) happens once before the measured loop, so it never enters
/// a per-request sample — the gate measures steady-state added latency on
/// established connections.
pub async fn drive_mitm(
    target: TlsTarget,
    conns: usize,
    duration: Duration,
    body_len: usize,
) -> Stats {
    let request: Arc<[u8]> = format!("GET / HTTP/1.1\r\nHost: {}\r\n\r\n", target.server_name())
        .into_bytes()
        .into();
    let want_body = body_len;
    let target = Arc::new(target);

    let start = Instant::now();
    let deadline = start + duration;

    let mut handles = Vec::with_capacity(conns);
    for _ in 0..conns {
        let request = Arc::clone(&request);
        let target = Arc::clone(&target);
        handles.push(tokio::spawn(async move {
            // 32 unmeasured round-trips establish the proxy's upstream
            // connection and warm caches before any sample is taken.
            const WARMUP: usize = 32;
            match establish_tls(&target).await {
                Some(conn) => drive_one_stream(conn, &request, want_body, WARMUP, deadline).await,
                None => Vec::new(),
            }
        }));
    }

    let mut all: Vec<u64> = Vec::new();
    for h in handles {
        if let Ok(samples) = h.await {
            all.extend(samples);
        }
    }
    let elapsed = start.elapsed();

    let reqs = all.len() as u64;
    let (p50_us, p99_us) = percentiles(&mut all);
    Stats {
        p50_us,
        p99_us,
        reqs,
        elapsed,
    }
}

/// Establishes one client-TLS session to `target`: for [`TlsTarget::ViaMitm`]
/// it first opens the `CONNECT` tunnel through the proxy. Returns `None` if
/// any step fails, so that connection simply contributes no samples.
async fn establish_tls(target: &TlsTarget) -> Option<TlsStream<TcpStream>> {
    match target {
        TlsTarget::Direct {
            addr,
            ca_der,
            server_name,
        } => {
            let conn = TcpStream::connect(addr).await.ok()?;
            let _ = conn.set_nodelay(true);
            client_tls(conn, ca_der, server_name).await
        }
        TlsTarget::ViaMitm {
            proxy,
            authority,
            ca_der,
            server_name,
        } => {
            let mut conn = TcpStream::connect(proxy).await.ok()?;
            let _ = conn.set_nodelay(true);
            let req = format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n\r\n");
            conn.write_all(req.as_bytes()).await.ok()?;
            read_connect_ok(&mut conn).await?;
            client_tls(conn, ca_der, server_name).await
        }
    }
}

/// Reads the proxy's `CONNECT` response head and checks it is a `200`.
async fn read_connect_ok(conn: &mut TcpStream) -> Option<()> {
    let mut carry: Vec<u8> = Vec::with_capacity(128);
    let mut buf = [0u8; 1024];
    let head_end = loop {
        if let Some(pos) = find_subslice(&carry, b"\r\n\r\n") {
            break pos + 4;
        }
        match conn.read(&mut buf).await {
            Ok(0) | Err(_) => return None,
            Ok(n) => carry.extend_from_slice(&buf[..n]),
        }
    };
    // The tunnel is byte-clean after the blank line; the echo upstream never
    // pre-sends, so there are no leftover bytes to carry into the TLS layer.
    let line = &carry[..head_end];
    line.windows(4).any(|w| w == b" 200").then_some(())
}

/// Completes a client TLS handshake over `stream`, trusting only `ca_der`,
/// presenting SNI `server_name` and `http/1.1` ALPN (ring provider).
async fn client_tls<S: AsyncRead + AsyncWrite + Unpin>(
    stream: S,
    ca_der: &[u8],
    server_name: &str,
) -> Option<TlsStream<S>> {
    let mut store = RootCertStore::empty();
    store.add(CertificateDer::from(ca_der.to_vec())).ok()?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut cfg = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .ok()?
        .with_root_certificates(store)
        .with_no_client_auth();
    cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
    let name = ServerName::try_from(server_name.to_string()).ok()?;
    TlsConnector::from(Arc::new(cfg))
        .connect(name, stream)
        .await
        .ok()
}
