//! In-process upstream servers for the proxy tests.

use std::collections::VecDeque;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crosswire_ca::{Ca, IssuerLimits, LeafIssuer};
use crosswire_h1::{Parser, ParserConfig, Progress};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use rustls::ServerConfig;
use rustls_pki_types::ServerName;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

/// Issues a leaf for whatever SNI the client sends (an IP SAN for
/// 127.0.0.1 without one: clients send no SNI for an IP literal), signed by
/// the upstream's own CA.
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

/// A TLS server on loopback with its own CA (not the proxy's).
pub struct TlsUpstream {
    /// Where it listens.
    pub addr: SocketAddr,
    /// The CA its leaves chain to.
    pub ca: Arc<Ca>,
    /// Every decrypted byte it received, across connections.
    pub received: Arc<Mutex<Vec<u8>>>,
    /// Connections whose handshake completed.
    pub connections: Arc<AtomicUsize>,
}

impl TlsUpstream {
    /// The listening port.
    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    /// The upstream CA certificate, DER.
    pub fn ca_der(&self) -> Vec<u8> {
        self.ca.cert_der()
    }
}

/// A TLS acceptor with a fresh upstream CA issuing per-SNI leaves.
fn tls_acceptor() -> (TlsAcceptor, Arc<Ca>) {
    let ca = Arc::new(Ca::generate("upstream-test").unwrap());
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

/// Starts a TLS echo server that accepts any number of connections and
/// echoes each one's bytes back until the client closes.
pub async fn spawn_tls_echo() -> TlsUpstream {
    let (acceptor, ca) = tls_acceptor();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let received = Arc::new(Mutex::new(Vec::new()));
    let connections = Arc::new(AtomicUsize::new(0));
    let (rec, conns) = (Arc::clone(&received), Arc::clone(&connections));
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let acceptor = acceptor.clone();
            let (rec, conns) = (Arc::clone(&rec), Arc::clone(&conns));
            tokio::spawn(async move {
                let Ok(mut tls) = acceptor.accept(tcp).await else {
                    return;
                };
                conns.fetch_add(1, Ordering::SeqCst);
                let mut buf = [0u8; 8192];
                loop {
                    match tls.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            rec.lock().unwrap().extend_from_slice(&buf[..n]);
                            if tls.write_all(&buf[..n]).await.is_err() || tls.flush().await.is_err()
                            {
                                break;
                            }
                        }
                    }
                }
                let _ = tls.shutdown().await;
            });
        }
    });
    TlsUpstream {
        addr,
        ca,
        received,
        connections,
    }
}

/// A plain TCP upstream replying with scripted bytes.
pub struct RawUpstream {
    /// Where it listens.
    pub addr: SocketAddr,
    /// Every byte it received, across connections, in arrival order.
    pub received: Arc<Mutex<Vec<u8>>>,
    /// Connections accepted.
    pub connections: Arc<AtomicUsize>,
}

impl RawUpstream {
    /// The listening port.
    pub fn port(&self) -> u16 {
        self.addr.port()
    }
}

/// Starts a plain TCP server that answers each complete request (parsed
/// with `crosswire-h1`) with the next entry of `script`, written in pieces
/// of `fragment` bytes when set. The script is shared by all connections.
/// A connection is closed after an entry that says `Connection: close` or
/// is HTTP/1.0, and when the script runs out.
pub async fn spawn_raw(script: Vec<Vec<u8>>, fragment: Option<usize>) -> RawUpstream {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let received = Arc::new(Mutex::new(Vec::new()));
    let connections = Arc::new(AtomicUsize::new(0));
    let script = Arc::new(Mutex::new(VecDeque::from(script)));
    let (rec, conns) = (Arc::clone(&received), Arc::clone(&connections));
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            conns.fetch_add(1, Ordering::SeqCst);
            let _ = tcp.set_nodelay(true);
            tokio::spawn(serve_script(
                tcp,
                Arc::clone(&script),
                Arc::clone(&rec),
                fragment,
            ));
        }
    });
    RawUpstream {
        addr,
        received,
        connections,
    }
}

/// As [`spawn_raw`], over TLS with the upstream's own CA.
pub async fn spawn_tls_scripted(script: Vec<Vec<u8>>, fragment: Option<usize>) -> TlsUpstream {
    let (acceptor, ca) = tls_acceptor();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let received = Arc::new(Mutex::new(Vec::new()));
    let connections = Arc::new(AtomicUsize::new(0));
    let script = Arc::new(Mutex::new(VecDeque::from(script)));
    let (rec, conns) = (Arc::clone(&received), Arc::clone(&connections));
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let _ = tcp.set_nodelay(true);
            let acceptor = acceptor.clone();
            let (script, rec, conns) = (Arc::clone(&script), Arc::clone(&rec), Arc::clone(&conns));
            tokio::spawn(async move {
                let Ok(tls) = acceptor.accept(tcp).await else {
                    return;
                };
                conns.fetch_add(1, Ordering::SeqCst);
                serve_script(tls, script, rec, fragment).await;
            });
        }
    });
    TlsUpstream {
        addr,
        ca,
        received,
        connections,
    }
}

/// A TLS upstream answering every request, on any number of connections,
/// with `200 OK` and `body` as `text/html` (keep-alive). Browsers fetch
/// more than the page, so a finite script would run dry.
pub async fn spawn_tls_html(body: &str) -> TlsUpstream {
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    spawn_tls_scripted(vec![resp.into_bytes(); 256], None).await
}

/// Whether a scripted response ends its connection.
fn closes_after(resp: &[u8]) -> bool {
    let head_end = resp
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .unwrap_or(resp.len());
    let head = String::from_utf8_lossy(&resp[..head_end]).to_ascii_lowercase();
    head.starts_with("http/1.0") || head.contains("\r\nconnection: close")
}

async fn serve_script<S: AsyncRead + AsyncWrite + Unpin>(
    mut io: S,
    script: Arc<Mutex<VecDeque<Vec<u8>>>>,
    received: Arc<Mutex<Vec<u8>>>,
    fragment: Option<usize>,
) {
    let mut parser = Parser::request(&ParserConfig::default());
    let mut carry: Vec<u8> = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = match io.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        received.lock().unwrap().extend_from_slice(&buf[..n]);
        carry.extend_from_slice(&buf[..n]);
        loop {
            let Ok(step) = parser.feed(&carry) else {
                return;
            };
            carry.drain(..step.consumed);
            match step.event {
                Progress::NeedMore => break,
                Progress::Done => {
                    parser.reset();
                    let next = script.lock().unwrap().pop_front();
                    let Some(resp) = next else {
                        let _ = io.shutdown().await;
                        return;
                    };
                    let piece = fragment.unwrap_or(resp.len()).max(1);
                    for part in resp.chunks(piece) {
                        if io.write_all(part).await.is_err() || io.flush().await.is_err() {
                            return;
                        }
                        if fragment.is_some() {
                            tokio::time::sleep(Duration::from_millis(1)).await;
                        }
                    }
                    if closes_after(&resp) {
                        let _ = io.shutdown().await;
                        return;
                    }
                }
                _ => {}
            }
        }
    }
}
