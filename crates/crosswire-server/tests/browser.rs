//! End-to-end tests of `crosswire browser`.
//!
//! The headless launch test is ignored by default and runs only when
//! `CROSSWIRE_BROWSER` names a Chromium-based binary (`just browser-test`).
#![allow(missing_docs)]

use std::io::{BufRead, BufReader};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::process::{Child, Command, Stdio};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use crosswire_ca::{Ca, IssuerLimits, LeafIssuer};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use rustls_pki_types::ServerName;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const BIN: &str = env!("CARGO_BIN_EXE_crosswire");

#[test]
fn browser_without_any_browser_prints_fallback_and_exits_3() {
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path().join("proj");
    let ca_dir = tmp.path().join("ca");
    let out = Command::new(BIN)
        .args([
            "browser",
            "--project",
            project.to_str().unwrap(),
            "--ca-dir",
            ca_dir.to_str().unwrap(),
            "--proxy",
            "127.0.0.1:65535",
            "--browser-path",
            "/nonexistent",
            "--url",
            "https://127.0.0.1:1/",
        ])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(3),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("http://crosswire/"), "stdout: {stdout}");
    assert!(stdout.contains("127.0.0.1:65535"), "stdout: {stdout}");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a Chromium binary: set CROSSWIRE_BROWSER (see `just browser-test`)"]
async fn browser_launches_headless_and_session_is_recorded() {
    let Some(browser) = std::env::var_os("CROSSWIRE_BROWSER").filter(|v| !v.is_empty()) else {
        eprintln!("CROSSWIRE_BROWSER is unset: skipping the browser launch test");
        return;
    };

    let upstream = spawn_tls_upstream().await;
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
    let startup = tokio::task::block_in_place(|| wait_serve_startup(&mut child.0));
    let token = token_from_bootstrap_line(&startup.bootstrap_line);
    let api = startup.api;
    let proxy = startup.proxy;

    let url = format!("https://127.0.0.1:{}/", upstream.port());
    let bout = Command::new(BIN)
        .args([
            "browser",
            "--project",
            project.to_str().unwrap(),
            "--ca-dir",
            ca_dir.to_str().unwrap(),
            "--proxy",
            &proxy.to_string(),
            "--headless",
            "--url",
            &url,
            "--",
            "--no-sandbox",
            "--disable-gpu",
            "--disable-dev-shm-usage",
            "--disable-extensions",
            "--disable-background-networking",
            "--disable-component-update",
            "--disable-sync",
            "--disable-default-apps",
            "--dump-dom",
            "--timeout=15000",
        ])
        .env("CROSSWIRE_BROWSER", &browser)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    assert!(
        bout.status.success(),
        "the launcher exited non-zero: {}",
        String::from_utf8_lossy(&bout.stderr)
    );

    let cookie = api_session_cookie(api, &token).await;
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let (st, _, body) = http(
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
        assert_eq!(st, 200, "counts: {}", String::from_utf8_lossy(&body));
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        if v["exchanges"].as_u64().unwrap_or(0) >= 1 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "no exchange was recorded via the API in time"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    sigint_and_wait(&mut child.0);
}

// -- the browser explicit path and detection ------------------------------
//
// `--browser-path` is honoured above; auto-detection is exercised by
// `just browser-test` on a machine with a real browser on PATH.

// -- test helpers (a self-contained raw HTTP/1 client and a TLS upstream) ---

struct Startup {
    proxy: SocketAddr,
    api: SocketAddr,
    bootstrap_line: String,
}

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

fn token_from_bootstrap_line(line: &str) -> String {
    line.split("#bootstrap=")
        .nth(1)
        .expect("a #bootstrap fragment")
        .split_whitespace()
        .next()
        .expect("a token")
        .to_string()
}

async fn api_session_cookie(api: SocketAddr, token: &str) -> String {
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
    hdrs.iter()
        .find(|(k, _)| k == "set-cookie")
        .map(|(_, v)| v.split(';').next().unwrap().to_string())
        .expect("a Set-Cookie header")
}

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
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("a header terminator");
    let head = String::from_utf8_lossy(&raw[..split]);
    let rbody = raw[split + 4..].to_vec();
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
    (status, hdrs, rbody)
}

struct Reap(Child);

impl Drop for Reap {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn sigint_and_wait(child: &mut Child) {
    let pid = child.id().to_string();
    assert!(Command::new("kill")
        .args(["-INT", &pid])
        .status()
        .unwrap()
        .success());
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "serve did not exit in time");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(status.code(), Some(0), "clean exit, got {status:?}");
}

/// Always serves a leaf for 127.0.0.1 (an IP SAN; no SNI is sent for IPs).
#[derive(Debug)]
struct LoopbackLeaf(Arc<CertifiedKey>);

impl ResolvesServerCert for LoopbackLeaf {
    fn resolve(&self, _hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        Some(Arc::clone(&self.0))
    }
}

struct Upstream {
    addr: SocketAddr,
}

impl Upstream {
    fn port(&self) -> u16 {
        self.addr.port()
    }
}

/// A TLS upstream answering every connection with a tiny HTML page.
async fn spawn_tls_upstream() -> Upstream {
    let ca = Arc::new(Ca::generate("browser-upstream").unwrap());
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
                let body = b"<html>hi</html>";
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = tls.write_all(resp.as_bytes()).await;
                let _ = tls.write_all(body).await;
                let _ = tls.shutdown().await;
            });
        }
    });
    Upstream { addr }
}
