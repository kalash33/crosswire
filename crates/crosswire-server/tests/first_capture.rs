//! The alpha.2 "two-minute first capture" DoD item, without a browser or any
//! network egress: `serve` is started, a session is minted from the 0600
//! bootstrap file, one request goes through the proxy to an in-process TLS
//! upstream, and the API must list the exchange in under 120 s.
//!
//! `scripts/first-capture.sh` measures the same flow with a real browser.
#![allow(missing_docs)]

use std::io::{BufRead, BufReader};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::process::{Child, Command, Stdio};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use crosswire_ca::{Ca, IssuerLimits, LeafIssuer};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use rustls_pki_types::{CertificateDer, ServerName};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const BIN: &str = env!("CARGO_BIN_EXE_crosswire");
const LIMIT: Duration = Duration::from_secs(120);

struct Reap(Child);

impl Drop for Reap {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Reads `serve`'s stdout until both listening lines, within 30 s.
fn wait_addrs(child: &mut Child) -> (SocketAddr, SocketAddr) {
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { return };
            let _ = tx.send(line);
        }
    });
    let deadline = Instant::now() + Duration::from_secs(30);
    let (mut proxy, mut api) = (None, None);
    while proxy.is_none() || api.is_none() {
        let left = deadline.saturating_duration_since(Instant::now());
        let line = rx.recv_timeout(left).expect("a serve startup line in time");
        if let Some(a) = line.strip_prefix("proxy listening on ") {
            proxy = Some(a.trim().parse().unwrap());
        } else if let Some(a) = line.strip_prefix("api listening on ") {
            api = Some(a.trim().parse().unwrap());
        }
    }
    (proxy.unwrap(), api.unwrap())
}

async fn http(
    addr: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<&[u8]>,
) -> (u16, Vec<(String, String)>, Vec<u8>) {
    let mut req = format!(
        "{method} {path} HTTP/1.1\r\nHost: localhost:{}\r\n",
        addr.port()
    );
    req.push_str("Connection: close\r\n");
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    if let Some(b) = body {
        req.push_str(&format!("Content-Length: {}\r\n", b.len()));
    }
    req.push_str("\r\n");
    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream.write_all(req.as_bytes()).await.unwrap();
    if let Some(b) = body {
        stream.write_all(b).await.unwrap();
    }
    stream.flush().await.unwrap();
    let mut raw = Vec::new();
    let mut tmp = [0u8; 8192];
    loop {
        match tokio::time::timeout(Duration::from_secs(10), stream.read(&mut tmp)).await {
            Ok(Ok(0)) | Ok(Err(_)) | Err(_) => break,
            Ok(Ok(n)) => raw.extend_from_slice(&tmp[..n]),
        }
    }
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("a header terminator");
    let head = String::from_utf8_lossy(&raw[..split]);
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let hdrs = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    (status, hdrs, raw[split + 4..].to_vec())
}

/// Always serves a leaf for 127.0.0.1 (an IP SAN; no SNI is sent for IPs).
#[derive(Debug)]
struct LoopbackLeaf(Arc<CertifiedKey>);

impl ResolvesServerCert for LoopbackLeaf {
    fn resolve(&self, _hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        Some(Arc::clone(&self.0))
    }
}

/// A TLS server answering every connection with `200 ok`.
async fn spawn_tls_upstream() -> SocketAddr {
    let ca = Arc::new(Ca::generate("first-capture-upstream").unwrap());
    let issuer = LeafIssuer::new(ca, IssuerLimits::default());
    let leaf = issuer
        .issue(&ServerName::IpAddress(
            IpAddr::V4(Ipv4Addr::LOCALHOST).into(),
        ))
        .unwrap();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let cfg = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_cert_resolver(Arc::new(LoopbackLeaf(leaf)));
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(cfg));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((tcp, _)) = listener.accept().await else {
                return;
            };
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                let Ok(mut tls) = acceptor.accept(tcp).await else {
                    return;
                };
                let mut head = Vec::new();
                let mut buf = [0u8; 4096];
                while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                    match tls.read(&mut buf).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => head.extend_from_slice(&buf[..n]),
                    }
                }
                let _ = tls
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                    )
                    .await;
                let _ = tls.shutdown().await;
            });
        }
    });
    addr
}

/// CONNECT through the proxy, trust the exported CA, GET `/`.
async fn fetch_via_proxy(proxy: SocketAddr, upstream: SocketAddr, ca_der: Vec<u8>) -> Vec<u8> {
    let mut tcp = TcpStream::connect(proxy).await.unwrap();
    let authority = format!("127.0.0.1:{}", upstream.port());
    tcp.write_all(format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        assert_eq!(tcp.read(&mut byte).await.unwrap(), 1, "proxy closed");
        head.push(byte[0]);
    }
    assert!(String::from_utf8_lossy(&head).starts_with("HTTP/1.1 200"));
    let mut roots = rustls::RootCertStore::empty();
    roots.add(CertificateDer::from(ca_der)).unwrap();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let cfg = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let connector = tokio_rustls::TlsConnector::from(Arc::new(cfg));
    let name = ServerName::IpAddress(IpAddr::V4(Ipv4Addr::LOCALHOST).into());
    let mut tls = connector.connect(name, tcp).await.unwrap();
    tls.write_all(
        format!("GET / HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\n\r\n").as_bytes(),
    )
    .await
    .unwrap();
    let mut resp = Vec::new();
    let _ = tls.read_to_end(&mut resp).await;
    resp
}

#[tokio::test(flavor = "multi_thread")]
async fn first_capture_under_two_minutes() {
    let upstream = spawn_tls_upstream().await;
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path().join("proj");
    let ca_dir = tmp.path().join("ca");
    let boot = tmp.path().join("boot");

    let start = Instant::now();
    let child = Command::new(BIN)
        .args([
            "serve",
            "--project",
            project.to_str().unwrap(),
            "--ca-dir",
            ca_dir.to_str().unwrap(),
            "--api-bind",
            "127.0.0.1:0",
            "--proxy-bind",
            "127.0.0.1:0",
            "--bootstrap-file",
            boot.to_str().unwrap(),
            "--allow",
            &format!("127.0.0.1:{}", upstream.port()),
            "--accept-invalid-cert",
            "127.0.0.1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut child = Reap(child);
    let (proxy, api) = tokio::task::block_in_place(|| wait_addrs(&mut child.0));

    // The token comes from the 0600 file (written before the lines above).
    let url = std::fs::read_to_string(&boot).unwrap();
    let token = url
        .split("#bootstrap=")
        .nth(1)
        .and_then(|t| t.split_whitespace().next())
        .expect("a bootstrap token in the file");
    let body = format!("{{\"bootstrap\":\"{token}\"}}");
    let (st, hdrs, _) = http(
        api,
        "POST",
        "/api/v1/session",
        &[("Content-Type", "application/json")],
        Some(body.as_bytes()),
    )
    .await;
    assert_eq!(st, 200, "the bootstrap token should create a session");
    let cookie = hdrs
        .iter()
        .find(|(k, _)| k == "set-cookie")
        .map(|(_, v)| v.split(';').next().unwrap().to_string())
        .expect("a Set-Cookie header");

    let pem = Command::new(BIN)
        .args([
            "ca",
            "export",
            "--format",
            "pem",
            "--ca-dir",
            ca_dir.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(pem.status.success());
    let (_, pem) = x509_parser::pem::parse_x509_pem(&pem.stdout).unwrap();
    let resp = fetch_via_proxy(proxy, upstream, pem.contents).await;
    assert!(
        String::from_utf8_lossy(&resp).starts_with("HTTP/1.1 200"),
        "response: {}",
        String::from_utf8_lossy(&resp)
    );

    let path = "/api/v1/exchanges?host_contains=127.0.0.1";
    loop {
        let (st, _, body) = http(
            api,
            "GET",
            path,
            &[
                ("Cookie", cookie.as_str()),
                ("Sec-Fetch-Site", "same-origin"),
            ],
            None,
        )
        .await;
        assert_eq!(st, 200, "exchanges: {}", String::from_utf8_lossy(&body));
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        if !v["items"].as_array().unwrap().is_empty() {
            break;
        }
        assert!(start.elapsed() < LIMIT, "no exchange recorded in time");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let elapsed = start.elapsed();
    eprintln!("first capture in {elapsed:?}");
    assert!(elapsed < LIMIT, "first capture took {elapsed:?}");
}
