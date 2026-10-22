//! In-process servers for the recorder integration tests.
//!
//! Copied (trimmed) from `crates/crosswire-client/tests/common/mod.rs`. Plan 3
//! will move the shared servers into a `crosswire-testing` dev crate; until
//! then the duplication is deliberate.
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

/// Reads from `s` until the request head's blank line has been seen.
async fn read_request_head<S: AsyncReadExt + Unpin>(s: &mut S) -> bool {
    let mut got = Vec::new();
    let mut buf = [0u8; 1024];
    while !got.windows(4).any(|w| w == b"\r\n\r\n") {
        match s.read(&mut buf).await {
            Ok(0) | Err(_) => return false,
            Ok(n) => got.extend_from_slice(&buf[..n]),
        }
    }
    true
}

/// TLS server for `example.test` with a leaf issued by the returned CA. It
/// accepts one connection, reads a request head, writes `response`, then
/// closes.
pub async fn spawn_tls_scripted(response: Vec<u8>) -> (Ca, SocketAddr) {
    let ca = Ca::generate("recorder-test").unwrap();
    let issuer = LeafIssuer::new(Arc::new(ca.clone()), IssuerLimits::default());
    let ck = issuer
        .issue(&ServerName::try_from("example.test".to_string()).unwrap())
        .unwrap();
    let cfg = ServerConfig::builder()
        .with_no_client_auth()
        .with_cert_resolver(Arc::new(Fixed(ck)));
    let acceptor = TlsAcceptor::from(Arc::new(cfg));
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move {
        if let Ok((s, _)) = l.accept().await {
            if let Ok(mut t) = acceptor.accept(s).await {
                if read_request_head(&mut t).await {
                    let _ = t.write_all(&response).await;
                    let _ = t.flush().await;
                }
                let _ = t.shutdown().await;
            }
        }
    });
    (ca, addr)
}

/// Plain server: accepts one connection, reads a request head, writes
/// `response`, then closes.
pub async fn spawn_plain_scripted(response: Vec<u8>) -> SocketAddr {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move {
        if let Ok((mut s, _)) = l.accept().await {
            if read_request_head(&mut s).await {
                let _ = s.write_all(&response).await;
            }
            let _ = s.shutdown().await;
        }
    });
    addr
}
