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

/// TLS echo server whose leaf for `name` is issued by `ca`. Lets a single
/// private CA stand up more than one server (each server accepts exactly one
/// connection), so the same CA/leaf can be driven down two trust paths.
pub async fn spawn_tls_echo_from_ca(ca: &Ca, name: &str) -> SocketAddr {
    spawn_tls(ca, name).await
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

/// Writes `bytes` cut at `split_at` offsets, pausing 20 ms between pieces.
async fn write_split<S: AsyncWriteExt + Unpin>(s: &mut S, bytes: &[u8], split_at: &[usize]) {
    let mut start = 0;
    for &cut in split_at.iter().chain(std::iter::once(&bytes.len())) {
        if s.write_all(&bytes[start..cut]).await.is_err() || s.flush().await.is_err() {
            return;
        }
        start = cut;
        if cut != bytes.len() {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }
}

/// Plain server: accepts one connection, reads a request head (ignoring any
/// body), writes `response` split at `split_at` with 20 ms pauses, then closes.
pub async fn spawn_scripted(response: &'static [u8], split_at: &[usize]) -> SocketAddr {
    let split_at = split_at.to_vec();
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move {
        if let Ok((mut s, _)) = l.accept().await {
            if read_request_head(&mut s).await {
                write_split(&mut s, response, &split_at).await;
            }
            let _ = s.shutdown().await;
        }
    });
    addr
}

/// Plain server: accepts one connection and answers each request head with the
/// next scripted response, keeping the connection open until the client closes.
pub async fn spawn_scripted_keepalive(responses: Vec<&'static [u8]>) -> SocketAddr {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move {
        if let Ok((mut s, _)) = l.accept().await {
            for r in responses {
                if !read_request_head(&mut s).await {
                    return;
                }
                if s.write_all(r).await.is_err() {
                    return;
                }
            }
            let mut buf = [0u8; 64];
            while matches!(s.read(&mut buf).await, Ok(n) if n > 0) {}
        }
    });
    addr
}

/// Plain server that accepts one connection, reads a request head, and never
/// answers.
pub async fn spawn_silent() -> SocketAddr {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move {
        if let Ok((mut s, _)) = l.accept().await {
            read_request_head(&mut s).await;
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        }
    });
    addr
}
