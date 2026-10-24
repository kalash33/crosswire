//! End-to-end tests of the `crosswire` binary.
#![allow(missing_docs)]

use std::io::{BufRead, BufReader};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::os::unix::fs::PermissionsExt;
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

#[test]
fn serve_refuses_a_ca_dir_inside_the_project() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join("proj")).unwrap();
    let proj = tmp.path().join("proj");
    // Spelled differently, and not yet existing: still inside the project.
    let sneaky = tmp.path().join("other").join("..").join("proj").join("ca");
    for ca in [proj.clone(), proj.join("keys").join("ca"), sneaky] {
        let out = run(&[
            "serve",
            "--project",
            proj.to_str().unwrap(),
            "--ca-dir",
            ca.to_str().unwrap(),
            "--proxy-bind",
            "127.0.0.1:0",
        ]);
        assert_eq!(out.status.code(), Some(2), "{ca:?}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains("--ca-dir"), "stderr: {stderr}");
    }
    // Refused before anything was created.
    assert_eq!(std::fs::read_dir(&proj).unwrap().count(), 0);
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

/// The four startup lines `serve` prints, parsed.
struct Startup {
    #[allow(dead_code)]
    proxy: SocketAddr,
    api: SocketAddr,
    bootstrap_line: String,
}

/// Reads `serve`'s stdout until it has the proxy address, the API address and
/// the bootstrap line, within 30 s.
fn wait_serve_startup(child: &mut Child) -> Startup {
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { return };
            let _ = tx.send(line);
        }
    });
    let deadline = Instant::now() + Duration::from_secs(30);
    let (mut proxy, mut api, mut bootstrap) = (None, None, None);
    while proxy.is_none() || api.is_none() || bootstrap.is_none() {
        let left = deadline.saturating_duration_since(Instant::now());
        let line = rx.recv_timeout(left).expect("a serve startup line in time");
        if let Some(a) = line.strip_prefix("proxy listening on ") {
            proxy = Some(a.trim().parse().unwrap());
        } else if let Some(a) = line.strip_prefix("api listening on ") {
            api = Some(a.trim().parse().unwrap());
        } else if line.starts_with("open http://localhost")
            || line.starts_with("bootstrap url written to")
        {
            bootstrap = Some(line);
        }
    }
    Startup {
        proxy: proxy.unwrap(),
        api: api.unwrap(),
        bootstrap_line: bootstrap.unwrap(),
    }
}

/// The bootstrap token out of the printed `open ...#bootstrap=<token> ...` line.
fn token_from_bootstrap_line(line: &str) -> String {
    let after = line
        .split("#bootstrap=")
        .nth(1)
        .expect("a #bootstrap fragment");
    after
        .split_whitespace()
        .next()
        .expect("a token")
        .to_string()
}

/// A minimal raw HTTP/1 client (no reqwest): one request, whole response.
async fn http(
    addr: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<&[u8]>,
) -> (u16, Vec<(String, String)>, Vec<u8>) {
    let mut req = format!("{method} {path} HTTP/1.1\r\n");
    if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("host")) {
        req.push_str(&format!("Host: localhost:{}\r\n", addr.port()));
    }
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
    parse_resp(&raw)
}

fn parse_resp(raw: &[u8]) -> (u16, Vec<(String, String)>, Vec<u8>) {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("a header terminator");
    let head = String::from_utf8_lossy(&raw[..split]);
    let body = raw[split + 4..].to_vec();
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    (status, headers, body)
}

/// Sends SIGINT and asserts a clean (0) exit within 10 s.
fn sigint_and_wait(child: &mut Child) {
    let pid = child.id().to_string();
    assert!(Command::new("kill")
        .args(["-INT", &pid])
        .status()
        .unwrap()
        .success());
    let status = wait_exit(child, Duration::from_secs(10));
    assert_eq!(status.code(), Some(0), "clean exit, got {status:?}");
}

/// The value after a `key:` line in command output (e.g. `secret:`).
fn field(out: &str, key: &str) -> String {
    out.lines()
        .find_map(|l| l.trim().strip_prefix(key))
        .map(|v| v.trim().to_string())
        .unwrap_or_else(|| panic!("no {key} line in:\n{out}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn serve_prints_api_line_and_bootstrap_once_and_session_works() {
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path().join("proj");
    let ca_dir = tmp.path().join("ca");
    let child = Command::new(BIN)
        .args([
            "serve",
            "--project",
            project.to_str().unwrap(),
            "--ca-dir",
            ca_dir.to_str().unwrap(),
            "--proxy-bind",
            "127.0.0.1:0",
            "--api-bind",
            "127.0.0.1:0",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut child = Reap(child);
    let startup = tokio::task::block_in_place(|| wait_serve_startup(&mut child.0));
    let token = token_from_bootstrap_line(&startup.bootstrap_line);
    let api = startup.api;

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
    assert!(cookie.starts_with("cw_sid_"), "cookie: {cookie}");

    let (st2, _, _) = http(
        api,
        "POST",
        "/api/v1/session",
        &[("Content-Type", "application/json")],
        Some(body.as_bytes()),
    )
    .await;
    assert_eq!(st2, 401, "the bootstrap token is single-use");

    let (st3, _, _) = http(
        api,
        "GET",
        "/api/v1/counts",
        &[
            ("Cookie", cookie.as_str()),
            ("Sec-Fetch-Site", "same-origin"),
        ],
        None,
    )
    .await;
    assert_eq!(st3, 200, "the session cookie authenticates");

    sigint_and_wait(&mut child.0);
}

#[tokio::test(flavor = "multi_thread")]
async fn bootstrap_file_is_0600_and_not_printed() {
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path().join("proj");
    let ca_dir = tmp.path().join("ca");
    let bfile = tmp.path().join("boot.url");
    let child = Command::new(BIN)
        .args([
            "serve",
            "--project",
            project.to_str().unwrap(),
            "--ca-dir",
            ca_dir.to_str().unwrap(),
            "--proxy-bind",
            "127.0.0.1:0",
            "--api-bind",
            "127.0.0.1:0",
            "--bootstrap-file",
            bfile.to_str().unwrap(),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut child = Reap(child);
    let startup = tokio::task::block_in_place(|| wait_serve_startup(&mut child.0));

    assert!(
        !startup.bootstrap_line.contains("#bootstrap="),
        "the token leaked to stdout: {}",
        startup.bootstrap_line
    );
    assert!(
        startup.bootstrap_line.contains("written to"),
        "line: {}",
        startup.bootstrap_line
    );

    let meta = std::fs::metadata(&bfile).unwrap();
    assert_eq!(meta.permissions().mode() & 0o777, 0o600, "0600 file");
    let content = std::fs::read_to_string(&bfile).unwrap();
    assert!(
        content.contains("#bootstrap="),
        "the file should hold the bootstrap url: {content}"
    );

    sigint_and_wait(&mut child.0);
}

#[tokio::test(flavor = "multi_thread")]
async fn token_create_list_revoke_cli() {
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path().join("proj");
    let ca_dir = tmp.path().join("ca");
    let p = project.to_str().unwrap();

    let s1 = String::from_utf8(ok(&[
        "token",
        "create",
        "--project",
        p,
        "--name",
        "alpha",
        "--scopes",
        "read,send",
    ]))
    .unwrap();
    let secret1 = field(&s1, "secret:");
    let id1 = field(&s1, "id:");
    assert!(secret1.starts_with("cwt_"), "secret: {secret1}");

    let s2 = String::from_utf8(ok(&[
        "token",
        "create",
        "--project",
        p,
        "--name",
        "beta",
        "--scopes",
        "read",
    ]))
    .unwrap();
    let secret2 = field(&s2, "secret:");
    let id2 = field(&s2, "id:");

    let list = String::from_utf8(ok(&["token", "list", "--project", p])).unwrap();
    assert!(list.contains(&id1) && list.contains(&id2), "list: {list}");
    assert!(
        list.contains("alpha") && list.contains("beta"),
        "list: {list}"
    );
    assert!(
        !list.contains("cwt_"),
        "the listing leaked a secret: {list}"
    );

    ok(&["token", "revoke", "--project", p, &id2]);

    let rot = String::from_utf8(ok(&["token", "rotate", "--project", p, &id1])).unwrap();
    let secret3 = field(&rot, "secret:");
    assert!(secret3.starts_with("cwt_"), "rotate secret: {secret3}");
    assert!(
        rot.contains(&format!("invalidated: {id1}")),
        "rotate output: {rot}"
    );

    let child = Command::new(BIN)
        .args([
            "serve",
            "--project",
            p,
            "--ca-dir",
            ca_dir.to_str().unwrap(),
            "--proxy-bind",
            "127.0.0.1:0",
            "--api-bind",
            "127.0.0.1:0",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut child = Reap(child);
    let startup = tokio::task::block_in_place(|| wait_serve_startup(&mut child.0));
    let api = startup.api;

    let (b1, b2, b3) = (
        format!("Bearer {secret1}"),
        format!("Bearer {secret2}"),
        format!("Bearer {secret3}"),
    );
    let (c1, _, _) = http(
        api,
        "GET",
        "/api/v1/counts",
        &[("Authorization", b1.as_str())],
        None,
    )
    .await;
    assert_eq!(c1, 401, "the rotated-out secret is rejected");
    let (c2, _, _) = http(
        api,
        "GET",
        "/api/v1/counts",
        &[("Authorization", b2.as_str())],
        None,
    )
    .await;
    assert_eq!(c2, 401, "the revoked secret is rejected");
    let (c3, _, _) = http(
        api,
        "GET",
        "/api/v1/counts",
        &[("Authorization", b3.as_str())],
        None,
    )
    .await;
    assert_eq!(c3, 200, "the rotated-in secret works");

    sigint_and_wait(&mut child.0);
}
