//! End-to-end tests of the `crosswire` binary.
#![allow(missing_docs)]

use std::io::{BufRead, BufReader};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use base64::Engine as _;
use crosswire_ca::{Ca, IssuerLimits, LeafIssuer};
use crosswire_model::MarkerKind;
use crosswire_store::{ExchangeFilter, Project, Store, StoreConfig};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use rustls_pki_types::{CertificateDer, ServerName};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const BIN: &str = env!("CARGO_BIN_EXE_crosswire");

fn run(args: &[&str]) -> Output {
    Command::new(BIN).args(args).output().unwrap()
}

fn ok(args: &[&str]) -> Vec<u8> {
    let out = run(args);
    assert!(
        out.status.success(),
        "crosswire {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}

fn pem_to_der(pem: &[u8]) -> Vec<u8> {
    let (_, pem) = x509_parser::pem::parse_x509_pem(pem).unwrap();
    assert_eq!(pem.label, "CERTIFICATE");
    pem.contents
}

#[test]
fn ca_export_der_matches_pem_and_spki() {
    let tmp = tempfile::tempdir().unwrap();
    let ca_dir = tmp.path().join("ca");
    let ca_dir = ca_dir.to_str().unwrap();
    let der_path = tmp.path().join("ca.der");
    let pem_path = tmp.path().join("ca.pem");

    ok(&[
        "ca",
        "export",
        "--format",
        "der",
        "--out",
        der_path.to_str().unwrap(),
        "--ca-dir",
        ca_dir,
    ]);
    ok(&[
        "ca",
        "export",
        "--format",
        "pem",
        "--out",
        pem_path.to_str().unwrap(),
        "--ca-dir",
        ca_dir,
    ]);
    let der = std::fs::read(&der_path).unwrap();
    let pem = std::fs::read(&pem_path).unwrap();
    assert_eq!(pem_to_der(&pem), der, "PEM and DER exports differ");

    // PEM to stdout is the same certificate.
    assert_eq!(
        pem_to_der(&ok(&[
            "ca", "export", "--format", "pem", "--ca-dir", ca_dir
        ])),
        der
    );

    let (_, cert) = x509_parser::parse_x509_certificate(&der).unwrap();
    let want = base64::engine::general_purpose::STANDARD
        .encode(Sha256::digest(cert.tbs_certificate.subject_pki.raw));
    let spki = String::from_utf8(ok(&["ca", "spki", "--ca-dir", ca_dir])).unwrap();
    assert_eq!(spki.trim(), want);
}

#[test]
fn serve_refuses_non_loopback_without_flag() {
    let tmp = tempfile::tempdir().unwrap();
    let out = run(&[
        "serve",
        "--project",
        tmp.path().join("proj").to_str().unwrap(),
        "--ca-dir",
        tmp.path().join("ca").to_str().unwrap(),
        "--proxy-bind",
        "0.0.0.0:0",
    ]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("--allow-non-loopback"), "stderr: {stderr}");
    // Config errors are caught before anything is created.
    assert!(!tmp.path().join("proj").exists());
}

/// Always serves a leaf for 127.0.0.1 (an IP SAN; no SNI is sent for IPs).
#[derive(Debug)]
struct LoopbackLeaf(Arc<CertifiedKey>);

impl ResolvesServerCert for LoopbackLeaf {
    fn resolve(&self, _hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        Some(Arc::clone(&self.0))
    }
}

/// A one-shot-per-connection TLS server answering `200 ok`.
async fn spawn_tls_upstream() -> SocketAddr {
    let ca = Arc::new(Ca::generate("cli-upstream").unwrap());
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

/// Kills the child if the test fails before it exits.
struct Reap(Child);

impl Drop for Reap {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Reads the child's stdout until the listening line, within 30 s.
fn wait_listening(child: &mut Child) -> SocketAddr {
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { return };
            let _ = tx.send(line);
        }
    });
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let line = rx.recv_timeout(left).expect("no listening line in time");
        if let Some(addr) = line.strip_prefix("proxy listening on ") {
            let addr: SocketAddr = addr.trim().parse().unwrap();
            let next = rx.recv_timeout(left).expect("no ca page line");
            assert_eq!(
                next,
                format!("ca page at http://crosswire/ (via the proxy) or http://{addr}/")
            );
            return addr;
        }
    }
}

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
    let head = String::from_utf8_lossy(&head);
    assert!(head.starts_with("HTTP/1.1 200"), "CONNECT answer: {head}");

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

fn wait_exit(child: &mut Child, within: Duration) -> std::process::ExitStatus {
    let deadline = Instant::now() + within;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        assert!(Instant::now() < deadline, "crosswire did not exit in time");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn audit_actions(project: &Path) -> String {
    std::fs::read_to_string(project.join("audit.jsonl")).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn serve_records_a_tls_fetch_and_closes_cleanly() {
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path().join("proj");
    let ca_dir = tmp.path().join("ca");
    let upstream = spawn_tls_upstream().await;

    let child = Command::new(BIN)
        .args([
            "serve",
            "--proxy-bind",
            "127.0.0.1:0",
            "--project",
            project.to_str().unwrap(),
            "--ca-dir",
            ca_dir.to_str().unwrap(),
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
    let proxy = tokio::task::block_in_place(|| wait_listening(&mut child.0));

    // The CA the running server uses, exported from the same directory.
    let pem = ok(&[
        "ca",
        "export",
        "--format",
        "pem",
        "--ca-dir",
        ca_dir.to_str().unwrap(),
    ]);
    let resp = fetch_via_proxy(proxy, upstream, pem_to_der(&pem)).await;
    let resp = String::from_utf8_lossy(&resp);
    assert!(resp.starts_with("HTTP/1.1 200"), "response: {resp}");
    assert!(resp.ends_with("ok"), "response: {resp}");

    let pid = child.0.id().to_string();
    let kill = Command::new("kill").args(["-INT", &pid]).status().unwrap();
    assert!(kill.success());
    let status = tokio::task::block_in_place(|| wait_exit(&mut child.0, Duration::from_secs(10)));
    assert_eq!(status.code(), Some(0), "exit status {status:?}");

    let store = Store::open(Project::open(&project).unwrap(), StoreConfig::default()).unwrap();
    let markers = store.markers(None, 100).unwrap().items;
    assert!(
        markers.iter().all(|m| m.kind() != MarkerKind::Gap),
        "unclean shutdown left a gap marker: {markers:?}"
    );
    let filter = ExchangeFilter {
        host: Some(format!("127.0.0.1:{}", upstream.port())),
        ..ExchangeFilter::default()
    };
    let rows = store.list(&filter, None, 10).unwrap().items;
    assert_eq!(rows.len(), 1, "rows: {rows:?}");
    assert_eq!(rows[0].status, Some(200));
    store.close().unwrap();

    let audit = audit_actions(&project);
    for action in [
        "\"ca.generate\"",
        "\"allow.add\"",
        "\"cert.override.config\"",
        "\"listener.bind\"",
        "\"listener.stop\"",
    ] {
        assert!(audit.contains(action), "audit lacks {action}: {audit}");
    }
}
