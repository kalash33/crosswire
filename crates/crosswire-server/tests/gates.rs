//! Spec §13 security gates that need the whole binary: the CA private key
//! never leaves, and every audited action is actually written.
//!
//! The API-surface gates live in `crates/crosswire-api/tests/gates.rs`; the
//! proxy/client gates are cited in `docs/security-gates.md`.
#![allow(missing_docs)]

use std::io::{BufRead, BufReader};
use std::net::SocketAddr;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const BIN: &str = env!("CARGO_BIN_EXE_crosswire");

// -- ca_key_never_leaves ----------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn ca_key_never_leaves() {
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path().join("proj");
    let ca_dir = tmp.path().join("ca");

    let mut child = Reap(spawn_serve(&project, &ca_dir, &[]));
    let startup = tokio::task::block_in_place(|| wait_serve_startup(&mut child.0));
    let api = startup.api;
    let cookie = api_session(api, &token_from_bootstrap_line(&startup.bootstrap_line)).await;

    // The CA private key, as written to disk (PKCS#8 PEM) and its DER bytes.
    let key_pem = std::fs::read(ca_dir.join("ca.key")).expect("the CA key file");
    let key_b64 = pem_body(&key_pem);
    let key_der = base64::engine::general_purpose::STANDARD
        .decode(&key_b64)
        .expect("the key PEM decodes");
    assert!(!key_der.is_empty(), "the key DER is non-empty");

    // No API response body carries the key. Walk every GET route the document
    // advertises (unauthenticated ones are reached without the cookie; the
    // cookie is harmless on them).
    for path in get_routes() {
        if path == "/api/v1/events" {
            continue; // the WebSocket route
        }
        let (status, _, body) = http(
            api,
            "GET",
            &concrete(&path),
            &[
                ("Cookie", cookie.as_str()),
                ("Sec-Fetch-Site", "same-origin"),
            ],
            None,
        )
        .await;
        assert_ne!(status, 0, "GET {path} produced no response");
        assert_absent(
            &body,
            &key_b64,
            key_der.as_slice(),
            &format!("GET {path} body"),
        );
    }

    // The OpenAPI document (committed and served) carries neither form.
    let committed = std::fs::read(openapi_snapshot_path()).expect("committed openapi");
    assert_absent(
        &committed,
        &key_b64,
        key_der.as_slice(),
        "committed OpenAPI",
    );

    // No file under the project directory carries the key (the key lives in a
    // separate ca-dir, which `serve` forbids placing inside a project).
    let mut files = 0usize;
    for file in walk(&project) {
        let bytes = std::fs::read(&file).unwrap_or_default();
        assert_absent(
            &bytes,
            &key_b64,
            key_der.as_slice(),
            &format!("project file {}", file.display()),
        );
        files += 1;
    }
    assert!(files > 0, "the project directory held files to scan");

    sigint_and_wait(&mut child.0);
}

/// The base64 body of a single-object PEM, with all whitespace removed.
fn pem_body(pem: &[u8]) -> String {
    String::from_utf8_lossy(pem)
        .lines()
        .filter(|l| !l.starts_with("-----"))
        .flat_map(|l| l.chars())
        .filter(|c| !c.is_whitespace())
        .collect()
}

/// Fails if `haystack` contains the key as base64 text or as raw DER bytes.
fn assert_absent(haystack: &[u8], key_b64: &str, key_der: &[u8], where_: &str) {
    let text = String::from_utf8_lossy(haystack);
    assert!(
        !text.contains(key_b64),
        "the CA private key (base64) leaked into {where_}"
    );
    assert!(
        !haystack.windows(key_der.len()).any(|w| w == key_der),
        "the CA private key (DER) leaked into {where_}"
    );
}

/// Every file under `dir`, recursively.
fn walk(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk(&path));
        } else {
            out.push(path);
        }
    }
    out
}

fn openapi_snapshot_path() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/api/openapi.json")
}

// -- audit_completeness -----------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn audit_completeness() {
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path().join("proj");
    let ca_dir = tmp.path().join("ca");
    let upstream = spawn_upstream().await;

    // --allow and --accept-invalid-cert make `serve` audit allow.add and
    // cert.override.config; a fresh ca-dir makes it audit ca.generate; binding
    // audits listener.bind.
    let allow = format!("127.0.0.1:{}", upstream.port());
    let mut child = Reap(spawn_serve(
        &project,
        &ca_dir,
        &["--allow", &allow, "--accept-invalid-cert", "127.0.0.1"],
    ));
    let startup = tokio::task::block_in_place(|| wait_serve_startup(&mut child.0));
    let api = startup.api;

    // session.bootstrap (ok).
    let cookie = api_session(api, &token_from_bootstrap_line(&startup.bootstrap_line)).await;
    let admin = [
        ("Cookie", cookie.as_str()),
        ("Sec-Fetch-Site", "same-origin"),
        ("Content-Type", "application/json"),
    ];

    // auth.fail: one bad bearer token.
    let (st, _, _) = http(
        api,
        "GET",
        "/api/v1/counts",
        &[("Authorization", "Bearer cwt_deadbeefdeadbeef")],
        None,
    )
    .await;
    assert_eq!(st, 401, "a bad bearer is rejected");

    // token.create, then token.revoke.
    let (st, _, body) = http(
        api,
        "POST",
        "/api/v1/tokens",
        &admin,
        Some(br#"{"name":"gate","scopes":["read"]}"#),
    )
    .await;
    assert_eq!(st, 200, "token create: {}", String::from_utf8_lossy(&body));
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let token_id = v["token"]["id"].as_str().unwrap().to_string();
    let (st, _, _) = http(
        api,
        "DELETE",
        &format!("/api/v1/tokens/{token_id}"),
        &admin,
        None,
    )
    .await;
    assert_eq!(st, 200, "token revoke");

    // scope.set: a scope that excludes the upstream (so the send is out of
    // scope), set before the send.
    let (st, _, body) = http(
        api,
        "PUT",
        "/api/v1/scope",
        &admin,
        Some(br#"{"include":[{"host":{"type":"exact","value":"other.test"}}],"exclude":[]}"#),
    )
    .await;
    assert_eq!(st, 200, "scope set: {}", String::from_utf8_lossy(&body));

    // intercept.enabled and intercept.rules.
    let (st, _, _) = http(
        api,
        "PUT",
        "/api/v1/intercept/enabled",
        &admin,
        Some(br#"{"enabled":true}"#),
    )
    .await;
    assert_eq!(st, 200, "intercept enabled");
    let (st, _, _) = http(
        api,
        "PUT",
        "/api/v1/intercept/rules",
        &admin,
        Some(br#"{"skip_extensions":[],"skip_out_of_scope":true,"overflow":"forward-newest","on_timeout":"forward"}"#),
    )
    .await;
    assert_eq!(st, 200, "intercept rules");

    // repeater.send.out_of_scope + repeater.send: one allowed out-of-scope send
    // to the upstream (reachable because of --allow).
    let raw = base64::engine::general_purpose::STANDARD
        .encode("GET / HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n");
    let send = format!(
        "{{\"host\":\"127.0.0.1\",\"port\":{},\"tls\":false,\"raw\":\"{raw}\",\"allow_out_of_scope\":true}}",
        upstream.port()
    );
    let (st, _, body) = http(
        api,
        "POST",
        "/api/v1/repeater/send",
        &admin,
        Some(send.as_bytes()),
    )
    .await;
    assert_eq!(st, 200, "repeater send: {}", String::from_utf8_lossy(&body));
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["scope_state"], "out", "the send is out of scope");

    // enforce_scope_strictly: after the send, since strict mode would block
    // the deliberately out-of-scope send above.
    let (st, _, _) = http(
        api,
        "PUT",
        "/api/v1/settings/enforce_scope_strictly",
        &admin,
        Some(br#"{"enabled":true}"#),
    )
    .await;
    assert_eq!(st, 200, "enforce_scope_strictly");

    // listener.stop is written on a clean shutdown.
    sigint_and_wait(&mut child.0);

    // browser.profile_reset, run after shutdown (it needs the project unlocked).
    let out = Command::new(BIN)
        .args([
            "browser",
            "reset-profile",
            "--project",
            project.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "browser reset-profile failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Every documented action was written exactly once.
    let log = std::fs::read_to_string(project.join("audit.jsonl")).unwrap();
    for action in [
        "session.bootstrap",
        "auth.fail",
        "token.create",
        "token.revoke",
        "scope.set",
        "enforce_scope_strictly",
        "intercept.enabled",
        "intercept.rules",
        "repeater.send",
        "repeater.send.out_of_scope",
        "allow.add",
        "cert.override.config",
        "browser.profile_reset",
        "listener.bind",
        "listener.stop",
        "ca.generate",
    ] {
        let needle = format!("\"{action}\"");
        let count = log.lines().filter(|l| l.contains(&needle)).count();
        assert_eq!(
            count, 1,
            "audit action {action} should appear exactly once (got {count})\n{log}"
        );
    }
}

// -- shared helpers (a self-contained raw HTTP/1 client and process driver) --

struct Reap(Child);

impl Drop for Reap {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn spawn_serve(project: &Path, ca_dir: &Path, extra: &[&str]) -> Child {
    let mut args = vec![
        "serve",
        "--project",
        project.to_str().unwrap(),
        "--ca-dir",
        ca_dir.to_str().unwrap(),
        "--proxy-bind",
        "127.0.0.1:0",
        "--api-bind",
        "127.0.0.1:0",
    ];
    args.extend_from_slice(extra);
    Command::new(BIN)
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap()
}

struct Startup {
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
    let (mut api, mut bootstrap) = (None, None);
    while api.is_none() || bootstrap.is_none() {
        let left = deadline.saturating_duration_since(Instant::now());
        let line = rx.recv_timeout(left).expect("a serve startup line in time");
        if let Some(a) = line.strip_prefix("api listening on ") {
            api = Some(a.trim().parse().unwrap());
        } else if line.starts_with("open http://localhost")
            || line.starts_with("bootstrap url written to")
        {
            bootstrap = Some(line);
        }
    }
    Startup {
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

async fn api_session(api: SocketAddr, token: &str) -> String {
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

/// The GET paths the committed OpenAPI document advertises.
fn get_routes() -> Vec<String> {
    let doc = serde_json::to_value(crosswire_api::document()).unwrap();
    let mut out = Vec::new();
    for (path, item) in doc["paths"].as_object().unwrap() {
        if item.as_object().unwrap().contains_key("get") {
            out.push(path.clone());
        }
    }
    out
}

/// Replaces `{...}` path templating with a concrete placeholder.
fn concrete(path: &str) -> String {
    let mut out = String::new();
    let mut in_brace = false;
    for ch in path.chars() {
        match ch {
            '{' => {
                in_brace = true;
                out.push_str("00000000000000000000000000");
            }
            '}' => in_brace = false,
            c if !in_brace => out.push(c),
            _ => {}
        }
    }
    out
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

/// A raw loopback upstream that answers each connection with a fixed `200 ok`.
struct Upstream {
    addr: SocketAddr,
}

impl Upstream {
    fn port(&self) -> u16 {
        self.addr.port()
    }
}

async fn spawn_upstream() -> Upstream {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let mut head = Vec::new();
                while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                    match sock.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => head.extend_from_slice(&buf[..n]),
                    }
                }
                let _ = sock
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                    )
                    .await;
                let _ = sock.flush().await;
            });
        }
    });
    Upstream { addr }
}
