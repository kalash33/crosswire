//! The embedded UI bundle, its CSP + per-response nonce, and the static-asset
//! auth class: non-`/api/v1` paths are served without a credential, while every
//! `/api/v1` route stays gated.
#![allow(missing_docs)]

use std::io::{BufRead, BufReader};
use std::net::SocketAddr;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const BIN: &str = env!("CARGO_BIN_EXE_crosswire");

/// Kills the child if the test fails before it exits.
struct Reap(Child);

impl Drop for Reap {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Spawns `serve` with ephemeral proxy and API ports.
fn spawn_serve(project: &str, ca_dir: &str) -> Child {
    Command::new(BIN)
        .args([
            "serve",
            "--project",
            project,
            "--ca-dir",
            ca_dir,
            "--proxy-bind",
            "127.0.0.1:0",
            "--api-bind",
            "127.0.0.1:0",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap()
}

/// The API address and the bootstrap line `serve` prints, read from its stdout
/// within 30 s.
fn wait_startup(child: &mut Child) -> (SocketAddr, String) {
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { return };
            let _ = tx.send(line);
        }
    });
    let deadline = Instant::now() + Duration::from_secs(30);
    let (mut api, mut boot) = (None, None);
    while api.is_none() || boot.is_none() {
        let left = deadline.saturating_duration_since(Instant::now());
        let line = rx.recv_timeout(left).expect("a serve startup line in time");
        if let Some(a) = line.strip_prefix("api listening on ") {
            api = Some(a.trim().parse().unwrap());
        } else if line.starts_with("open http://localhost") {
            boot = Some(line);
        }
    }
    (api.unwrap(), boot.unwrap())
}

/// The bootstrap token out of the `open ...#bootstrap=<token> ...` line.
fn token_from(line: &str) -> String {
    line.split("#bootstrap=")
        .nth(1)
        .and_then(|a| a.split_whitespace().next())
        .expect("a #bootstrap token")
        .to_string()
}

/// Sends SIGINT and waits for a clean (0) exit.
fn sigint_and_wait(child: &mut Child) {
    let pid = child.id().to_string();
    assert!(Command::new("kill")
        .args(["-INT", &pid])
        .status()
        .unwrap()
        .success());
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert_eq!(status.code(), Some(0), "clean exit, got {status:?}");
            return;
        }
        assert!(Instant::now() < deadline, "crosswire did not exit in time");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// A minimal raw HTTP/1 client: one request, whole response. The `Host`
/// defaults to `localhost:<port>` (a loopback name the gate accepts).
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

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
}

/// Exchanges a bootstrap token for a session cookie.
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

/// The nonce inside a `style-src … 'nonce-<n>'` directive of a CSP header.
fn nonce_from_style_src(csp: &str) -> String {
    let after = csp.split("'nonce-").nth(1).expect("a 'nonce-' token");
    after
        .split('\'')
        .next()
        .expect("a closing quote")
        .to_string()
}

/// The `content` of the `<meta name="csp-nonce" content="<n>">` tag.
fn nonce_from_meta(html: &str) -> String {
    let after = html
        .split("name=\"csp-nonce\"")
        .nth(1)
        .expect("a csp-nonce meta tag");
    let after = after
        .split("content=\"")
        .nth(1)
        .expect("a content attribute");
    after
        .split('"')
        .next()
        .expect("a closing quote")
        .to_string()
}

/// A real file under the committed bundle's `assets/` directory (the hashed
/// names change when the UI source changes, so the test discovers one).
fn a_bundle_css() -> String {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/ui/assets");
    std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("reading {}: {e}", dir.display()))
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .find(|n| n.ends_with(".css"))
        .expect("a .css file in the committed bundle")
}

#[tokio::test(flavor = "multi_thread")]
async fn serves_ui_bundle_with_csp_and_keeps_api_gated() {
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path().join("proj");
    let ca_dir = tmp.path().join("ca");
    let mut child = Reap(spawn_serve(
        project.to_str().unwrap(),
        ca_dir.to_str().unwrap(),
    ));
    let (api, boot) = tokio::task::block_in_place(|| wait_startup(&mut child.0));

    // GET / -> 200, an HTML body, the UI CSP, and nosniff.
    let (st, hdrs, body) = http(api, "GET", "/", &[], None).await;
    assert_eq!(st, 200, "GET / should serve the SPA shell");
    let ctype = header(&hdrs, "content-type").unwrap_or("");
    assert!(ctype.starts_with("text/html"), "content-type: {ctype}");
    let text = String::from_utf8_lossy(&body);
    assert!(
        text.contains("<div id=\"app\">"),
        "not the SPA HTML: {text}"
    );
    let csp = header(&hdrs, "content-security-policy").expect("a CSP header");
    assert!(
        csp.contains("default-src 'self'"),
        "CSP lacks default-src 'self': {csp}"
    );
    assert!(csp.contains("style-src 'self' 'nonce-"), "CSP nonce: {csp}");
    // The other required UI CSP directives must all be present.
    for directive in [
        "frame-src http://127.0.0.1:",
        "object-src 'none'",
        "base-uri 'none'",
        "frame-ancestors 'none'",
    ] {
        assert!(csp.contains(directive), "CSP lacks {directive}: {csp}");
    }
    assert_eq!(header(&hdrs, "x-content-type-options"), Some("nosniff"));
    // The per-response nonce is also exposed to the app in the HTML.
    assert!(
        text.contains("meta name=\"csp-nonce\""),
        "the nonce meta is missing: {text}"
    );
    // Lock the CodeMirror coupling: the nonce the browser enforces (the
    // `style-src 'nonce-<n>'` of the CSP header) must be the exact nonce handed
    // to the app (the `<meta name="csp-nonce" content="<n>">` in the body).
    let csp_nonce = nonce_from_style_src(csp);
    let meta_nonce = nonce_from_meta(&text);
    assert_eq!(
        meta_nonce, csp_nonce,
        "the meta nonce must equal the CSP style-src nonce"
    );

    // The nonce is fresh per response: a second GET / differs.
    let (_, hdrs2, _) = http(api, "GET", "/", &[], None).await;
    assert_ne!(
        header(&hdrs2, "content-security-policy"),
        Some(csp),
        "the CSP nonce must be fresh per response"
    );

    // GET /assets/<real file> -> 200 with the right content-type.
    let css = a_bundle_css();
    let (sa, ha, _) = http(api, "GET", &format!("/assets/{css}"), &[], None).await;
    assert_eq!(sa, 200, "GET /assets/{css} should serve the bundle file");
    assert_eq!(
        header(&ha, "content-type"),
        Some("text/css"),
        "css content-type"
    );
    assert_eq!(header(&ha, "x-content-type-options"), Some("nosniff"));

    // GET /api/v1/counts with no credential -> 401 missing_credential (the API
    // stays gated; non-`/api/v1` static paths do not).
    let (sc, _, cb) = http(api, "GET", "/api/v1/counts", &[], None).await;
    assert_eq!(sc, 401, "the API must stay gated");
    let v: serde_json::Value = serde_json::from_slice(&cb).unwrap();
    assert_eq!(v["error"]["code"], "missing_credential");

    // GET /nope (unknown UI path) -> the index.html fallback with 200.
    let (sn, hn, nb) = http(api, "GET", "/nope", &[], None).await;
    assert_eq!(sn, 200, "an unknown UI path falls back to the SPA shell");
    assert!(
        header(&hn, "content-type")
            .unwrap_or("")
            .starts_with("text/html"),
        "fallback content-type"
    );
    assert!(
        String::from_utf8_lossy(&nb).contains("<div id=\"app\">"),
        "the fallback should be the SPA shell"
    );

    // An authenticated, unknown /api/v1 path reaches the UI fallback but must
    // never become HTML: it is a 404, not the SPA shell.
    let cookie = api_session(api, &token_from(&boot)).await;
    let auth = [
        ("Cookie", cookie.as_str()),
        ("Sec-Fetch-Site", "same-origin"),
    ];
    let (su, hu, ub) = http(api, "GET", "/api/v1/does-not-exist", &auth, None).await;
    assert_eq!(su, 404, "an unknown API path is 404");
    assert!(
        !header(&hu, "content-type")
            .unwrap_or("")
            .starts_with("text/html"),
        "an API 404 must not be HTML"
    );
    assert!(
        !String::from_utf8_lossy(&ub).contains("<div id=\"app\">"),
        "an API 404 must not be the SPA shell"
    );

    sigint_and_wait(&mut child.0);
}
