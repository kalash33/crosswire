//! Shared harness for the proxy integration tests: a proxy on an ephemeral
//! loopback port with a fresh CA, an in-memory sink and raw TCP helpers.
#![allow(dead_code, missing_docs)]

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crosswire_ca::{Ca, IssuerLimits, LeafIssuer};
use crosswire_client::{DestinationGate, RustlsBackend, Transport};
use crosswire_model::{Exchange, ExchangeSink, Marker, SinkError};
use crosswire_proxy::{Counters, Handler, NoopHandler, Proxy, ProxyConfig, ProxyDeps, ProxyLimits};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::watch;
use tokio::task::JoinHandle;

/// Collects everything the proxy records.
#[derive(Default)]
pub struct TestSink {
    pub exchanges: Mutex<Vec<Exchange>>,
    pub markers: Mutex<Vec<Marker>>,
}

impl ExchangeSink for TestSink {
    fn submit(&self, exchange: Exchange) -> Result<(), SinkError> {
        self.exchanges.lock().unwrap().push(exchange);
        Ok(())
    }

    fn submit_marker(&self, marker: Marker) -> Result<(), SinkError> {
        self.markers.lock().unwrap().push(marker);
        Ok(())
    }
}

/// Stops the proxy when told to, or when dropped.
pub struct ShutdownHandle {
    tx: watch::Sender<bool>,
    task: Option<JoinHandle<()>>,
}

impl ShutdownHandle {
    /// Signals shutdown and waits for `Proxy::run` to return.
    pub async fn shutdown(mut self) {
        let _ = self.tx.send(true);
        if let Some(task) = self.task.take() {
            task.await.unwrap();
        }
    }
}

impl Drop for ShutdownHandle {
    fn drop(&mut self) {
        let _ = self.tx.send(true);
    }
}

/// A running proxy.
pub struct ProxyHarness {
    pub addr: SocketAddr,
    pub shutdown: ShutdownHandle,
    pub sink: Arc<TestSink>,
    pub counters: Arc<Counters>,
    pub ca: Arc<Ca>,
}

/// Dependencies for a proxy with `ca`, recording into `sink`.
pub fn deps(ca: &Arc<Ca>, sink: Arc<TestSink>, handler: Arc<dyn Handler>) -> ProxyDeps {
    let issuer = Arc::new(LeafIssuer::new(Arc::clone(ca), IssuerLimits::default()));
    // The in-process upstream (and its CA and gate entry) arrive with Task 4.
    let backend = Arc::new(RustlsBackend::with_extra_roots(Vec::new()));
    let gate = Arc::new(DestinationGate::new(Vec::new()));
    ProxyDeps {
        issuer,
        ca_der: ca.cert_der(),
        ca_pem: ca.cert_pem(),
        ca_spki: ca.spki_sha256_base64(),
        transport: Arc::new(Transport::new(backend, gate)),
        sink,
        handler,
    }
}

/// A loopback config on an ephemeral port.
pub fn loopback_config() -> ProxyConfig {
    ProxyConfig::new("127.0.0.1:0".parse().unwrap())
}

impl ProxyHarness {
    /// Binds and runs a proxy with `cfg` and `handler`.
    pub async fn start(cfg: ProxyConfig, handler: Arc<dyn Handler>) -> ProxyHarness {
        let ca = Arc::new(Ca::generate("proxy-test").unwrap());
        let sink = Arc::new(TestSink::default());
        let proxy = Proxy::bind(cfg, deps(&ca, Arc::clone(&sink), handler))
            .await
            .unwrap();
        let addr = proxy.local_addr();
        let counters = proxy.counters();
        let (tx, rx) = watch::channel(false);
        let task = tokio::spawn(async move {
            proxy.run(rx).await.unwrap();
        });
        ProxyHarness {
            addr,
            shutdown: ShutdownHandle {
                tx,
                task: Some(task),
            },
            sink,
            counters,
            ca,
        }
    }

    /// A loopback proxy with default config except for `limits`.
    pub async fn start_with_limits(limits: ProxyLimits) -> ProxyHarness {
        let mut cfg = loopback_config();
        cfg.limits = limits;
        ProxyHarness::start(cfg, Arc::new(NoopHandler)).await
    }

    /// A loopback proxy with the defaults.
    pub async fn start_default() -> ProxyHarness {
        ProxyHarness::start(loopback_config(), Arc::new(NoopHandler)).await
    }
}

const READ_TIMEOUT: Duration = Duration::from_secs(5);

/// Opens a TCP connection to `addr`.
pub async fn connect_raw(addr: SocketAddr) -> TcpStream {
    let s = TcpStream::connect(addr).await.unwrap();
    s.set_nodelay(true).unwrap();
    s
}

/// Reads until EOF (or a reset), failing after five seconds.
pub async fn read_until_close(s: &mut TcpStream) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = [0u8; 8192];
    tokio::time::timeout(READ_TIMEOUT, async {
        loop {
            match s.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => out.extend_from_slice(&buf[..n]),
            }
        }
    })
    .await
    .expect("connection was not closed in time");
    out
}

/// Reads until `needle` appears, failing after five seconds.
pub async fn read_until(s: &mut TcpStream, needle: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = [0u8; 8192];
    tokio::time::timeout(READ_TIMEOUT, async {
        while !out.windows(needle.len()).any(|w| w == needle) {
            match s.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => out.extend_from_slice(&buf[..n]),
            }
        }
    })
    .await
    .expect("needle not seen in time");
    out
}

/// Connects, writes `bytes`, and returns everything read until the proxy
/// closes the connection.
pub async fn send_and_read_until_close(addr: SocketAddr, bytes: &[u8]) -> Vec<u8> {
    let mut s = connect_raw(addr).await;
    s.write_all(bytes).await.unwrap();
    read_until_close(&mut s).await
}

/// Splits a response into its head (through the blank line) and body.
pub fn split_response(resp: &[u8]) -> (String, Vec<u8>) {
    let end = resp
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map_or(resp.len(), |i| i + 4);
    (
        String::from_utf8_lossy(&resp[..end]).into_owned(),
        resp[end..].to_vec(),
    )
}
