//! In-process echo servers for transport tests. Each accepts one connection.
#![allow(dead_code, missing_docs)]

use std::net::SocketAddr;
use std::sync::Arc;

use crosswire_ca::{Ca, IssuerLimits, LeafIssuer};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use rustls::ServerConfig;
use rustls_pki_types::ServerName;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

#[derive(Debug)]
struct Fixed(Arc<CertifiedKey>);

impl ResolvesServerCert for Fixed {
    fn resolve(&self, _hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        Some(Arc::clone(&self.0))
    }
}

async fn spawn_tls(ca: &Ca, name: &str) -> SocketAddr {
    let issuer = LeafIssuer::new(Arc::new(ca.clone()), IssuerLimits::default());
    let ck = issuer
        .issue(&ServerName::try_from(name.to_string()).unwrap())
        .unwrap();
    let mut cfg = ServerConfig::builder()
        .with_no_client_auth()
        .with_cert_resolver(Arc::new(Fixed(ck)));
    cfg.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    let acceptor = TlsAcceptor::from(Arc::new(cfg));
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move {
        if let Ok((s, _)) = l.accept().await {
            if let Ok(mut t) = acceptor.accept(s).await {
                echo(&mut t).await;
            }
        }
    });
    addr
}

async fn echo<S: AsyncReadExt + AsyncWriteExt + Unpin>(s: &mut S) {
    let mut buf = [0u8; 1024];
    while let Ok(n) = s.read(&mut buf).await {
        if n == 0 || s.write_all(&buf[..n]).await.is_err() {
            break;
        }
        let _ = s.flush().await;
    }
}

/// TLS echo server whose leaf for `example.test` is issued by the returned CA.
pub async fn spawn_tls_echo_with_ca() -> (Ca, SocketAddr) {
    let ca = Ca::generate("transport-test").unwrap();
    let addr = spawn_tls(&ca, "example.test").await;
    (ca, addr)
}

/// TLS echo server whose leaf for `bad.test` comes from an unrelated CA.
pub async fn spawn_tls_echo_self_signed() -> SocketAddr {
    let other = Ca::generate("untrusted").unwrap();
    spawn_tls(&other, "bad.test").await
}

/// Plain TCP echo server.
pub async fn spawn_plain_echo() -> SocketAddr {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move {
        if let Ok((mut s, _)) = l.accept().await {
            echo(&mut s).await;
        }
    });
    addr
}
