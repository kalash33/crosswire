//! In-process upstream servers for the proxy tests.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use crosswire_ca::{Ca, IssuerLimits, LeafIssuer};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use rustls::ServerConfig;
use rustls_pki_types::ServerName;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

/// Issues a leaf for whatever SNI the client sends (`localhost` without
/// one), signed by the upstream's own CA.
struct PerSni(LeafIssuer);

impl std::fmt::Debug for PerSni {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PerSni")
    }
}

impl ResolvesServerCert for PerSni {
    fn resolve(&self, hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        let name = hello.server_name().unwrap_or("localhost").to_string();
        self.0.issue(&ServerName::try_from(name).ok()?).ok()
    }
}

/// A TLS echo server on loopback with its own CA (not the proxy's).
pub struct TlsUpstream {
    /// Where it listens.
    pub addr: SocketAddr,
    /// The CA its leaves chain to.
    pub ca: Arc<Ca>,
    /// Every decrypted byte it received, across connections.
    pub received: Arc<Mutex<Vec<u8>>>,
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

/// Starts a TLS echo server that accepts any number of connections and
/// echoes each one's bytes back until the client closes.
pub async fn spawn_tls_echo() -> TlsUpstream {
    let ca = Arc::new(Ca::generate("upstream-test").unwrap());
    let issuer = LeafIssuer::new(Arc::clone(&ca), IssuerLimits::default());
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut cfg = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_cert_resolver(Arc::new(PerSni(issuer)));
    cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
    let acceptor = TlsAcceptor::from(Arc::new(cfg));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let received = Arc::new(Mutex::new(Vec::new()));
    let rec = Arc::clone(&received);
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let acceptor = acceptor.clone();
            let rec = Arc::clone(&rec);
            tokio::spawn(async move {
                let Ok(mut tls) = acceptor.accept(tcp).await else {
                    return;
                };
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
    TlsUpstream { addr, ca, received }
}
