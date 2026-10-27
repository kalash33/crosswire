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
use std::sync::{mpsc, PoisonError};
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

// -- idle_egress (real binary) ---------------------------------------------

/// Gate #5 over the real `crosswire serve` binary: with the API open, a session
/// established and the event stream connected, the process makes no outbound
/// (non-loopback) connection while idle. Linux only: the probe reads
/// `/proc/<pid>/{fd,net/*}`; elsewhere the test skips with a notice.
#[tokio::test(flavor = "multi_thread")]
async fn real_serve_makes_no_outbound_connection_while_idle() {
    if !Path::new("/proc/self/net/tcp").exists() {
        eprintln!("SKIP real_serve_makes_no_outbound_connection_while_idle: no /proc (non-Linux)");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path().join("proj");
    let ca_dir = tmp.path().join("ca");

    let mut child = Reap(spawn_serve(&project, &ca_dir, &[]));
    let startup = tokio::task::block_in_place(|| wait_serve_startup(&mut child.0));
    let api = startup.api;
    let pid = child.0.id();
    let cookie = api_session(api, &token_from_bootstrap_line(&startup.bootstrap_line)).await;

    // Connect the event stream (a raw WebSocket upgrade) and keep it open.
    let mut ws = TcpStream::connect(api).await.unwrap();
    let req = format!(
        "GET /api/v1/events HTTP/1.1\r\nHost: localhost:{}\r\nConnection: Upgrade\r\n\
         Upgrade: websocket\r\nSec-WebSocket-Version: 13\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nCookie: {cookie}\r\n\
         Sec-Fetch-Site: same-origin\r\nOrigin: http://localhost:{}\r\n\r\n",
        api.port(),
        api.port()
    );
    ws.write_all(req.as_bytes()).await.unwrap();
    let mut buf = [0u8; 1024];
    let n = tokio::time::timeout(Duration::from_secs(10), ws.read(&mut buf))
        .await
        .expect("the upgrade response in time")
        .unwrap();
    let head = String::from_utf8_lossy(&buf[..n]).into_owned();
    assert!(
        head.starts_with("HTTP/1.1 101"),
        "event stream upgrade: {head}"
    );

    let before = outbound_connections(pid).expect("the serve child's /proc entry");
    tokio::time::sleep(Duration::from_secs(6)).await;
    let after = outbound_connections(pid).expect("the serve child's /proc entry");
    assert!(
        before.is_empty() && after.is_empty(),
        "idle serve made outbound connections: before={before:?} after={after:?}"
    );

    drop(ws);
    sigint_and_wait(&mut child.0);
}

/// The process's outbound sockets: established non-loopback TCP connections and
/// connected non-loopback UDP sockets, found by matching the socket inodes in
/// `/proc/<pid>/fd` against the namespace's `/proc/<pid>/net/{tcp,tcp6,udp,udp6}`
/// rows. `None` when the process's `/proc` entry cannot be read.
fn outbound_connections(pid: u32) -> Option<Vec<String>> {
    let mut inodes = std::collections::HashSet::new();
    for entry in std::fs::read_dir(format!("/proc/{pid}/fd")).ok()? {
        let Ok(entry) = entry else { continue };
        let Ok(target) = std::fs::read_link(entry.path()) else {
            continue;
        };
        let target = target.to_string_lossy().into_owned();
        if let Some(i) = target
            .strip_prefix("socket:[")
            .and_then(|t| t.strip_suffix(']'))
        {
            inodes.insert(i.to_string());
        }
    }
    let mut out = Vec::new();
    for (table, is_tcp) in [
        ("tcp", true),
        ("tcp6", true),
        ("udp", false),
        ("udp6", false),
    ] {
        let text = std::fs::read_to_string(format!("/proc/{pid}/net/{table}")).ok()?;
        out.extend(outbound_rows(&text, is_tcp, &inodes));
    }
    Some(out)
}

/// Parse one `/proc/net/{tcp,udp}[6]` table, keeping rows owned by `inodes`
/// whose remote is non-loopback and connected (TCP `st` 01; UDP a non-zero
/// remote address).
fn outbound_rows(
    text: &str,
    is_tcp: bool,
    inodes: &std::collections::HashSet<String>,
) -> Vec<String> {
    let mut out = Vec::new();
    for row in text.lines().skip(1) {
        let f: Vec<&str> = row.split_whitespace().collect();
        // sl local rem st tx:rx tr:when retrnsmt uid timeout inode ...
        if f.len() < 10 || !inodes.contains(f[9]) {
            continue;
        }
        let Some(remote) = parse_proc_addr(f[2]) else {
            continue;
        };
        let connected = if is_tcp {
            f[3] == "01"
        } else {
            !remote.0.is_unspecified()
        };
        if connected && !remote.0.is_loopback() {
            out.push(format!("{} -> {}:{}", f[1], remote.0, remote.1));
        }
    }
    out
}

/// `HEXIP:HEXPORT` as printed by the kernel: the address is a sequence of
/// native-endian (little-endian on every CI target) 32-bit words.
fn parse_proc_addr(s: &str) -> Option<(std::net::IpAddr, u16)> {
    let (ip, port) = s.split_once(':')?;
    let port = u16::from_str_radix(port, 16).ok()?;
    let mut bytes = Vec::new();
    for word in 0..ip.len() / 8 {
        let w = u32::from_str_radix(ip.get(word * 8..word * 8 + 8)?, 16).ok()?;
        bytes.extend_from_slice(&w.to_le_bytes());
    }
    let addr = match bytes.len() {
        4 => std::net::IpAddr::from(<[u8; 4]>::try_from(bytes).ok()?),
        16 => {
            let v6 = std::net::Ipv6Addr::from(<[u8; 16]>::try_from(bytes).ok()?);
            match v6.to_ipv4_mapped() {
                Some(v4) => std::net::IpAddr::V4(v4),
                None => std::net::IpAddr::V6(v6),
            }
        }
        _ => return None,
    };
    Some((addr, port))
}

/// The probe's parser is platform-independent; pin it against kernel-format
/// rows so the counting path is exercised on every host, not only Linux.
#[test]
fn outbound_probe_parses_kernel_rows() {
    let inodes: std::collections::HashSet<String> =
        ["111", "222", "333", "444"].map(String::from).into();
    let tcp = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n\
 0: 0100007F:1F90 0100007F:C350 01 00000000:00000000 00:00000000 00000000  1000        0 111 1 0 100 0 0 10 0\n\
 1: 0F02000A:C351 2201A8C0:01BB 01 00000000:00000000 00:00000000 00000000  1000        0 222 1 0 100 0 0 10 0\n\
 2: 0F02000A:C352 2201A8C0:01BB 06 00000000:00000000 00:00000000 00000000  1000        0 333 1 0 100 0 0 10 0\n\
 3: 0F02000A:C353 2201A8C0:01BB 01 00000000:00000000 00:00000000 00000000  1000        0 999 1 0 100 0 0 10 0\n";
    let rows = outbound_rows(tcp, true, &inodes);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert!(rows[0].contains("192.168.1.34:443"), "{rows:?}");

    let tcp6 = "  sl  local_address rem_address st tx_queue rx_queue tr tm->when retrnsmt uid timeout inode\n\
 0: 00000000000000000000000001000000:1F90 00000000000000000000000001000000:C350 01 0:0 0:0 0 1000 0 111 1\n\
 1: 0000000000000000FFFF00000100007F:1F90 0000000000000000FFFF00000100007F:C350 01 0:0 0:0 0 1000 0 222 1\n\
 2: 00000000000000000000000001000000:C351 B80D0120000000000000000001000000:01BB 01 0:0 0:0 0 1000 0 333 1\n";
    let rows = outbound_rows(tcp6, true, &inodes);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert!(rows[0].contains("2001:db8::1:443"), "{rows:?}");

    let udp = "  sl  local_address rem_address st tx_queue rx_queue tr tm->when retrnsmt uid timeout inode\n\
 0: 0F02000A:C351 00000000:0000 07 0:0 0:0 0 1000 0 111 2\n\
 1: 0F02000A:C352 08080808:0035 01 0:0 0:0 0 1000 0 444 2\n";
    let rows = outbound_rows(udp, false, &inodes);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert!(rows[0].contains("8.8.8.8:53"), "{rows:?}");
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

// -- redaction_golden -------------------------------------------------------

/// §13 gate #14 / §11.4: over a real seeded `serve`, the REDACTED copy-as-curl
/// (both shells), the REDACTED copy-raw, the process stdout+stderr and
/// `audit.jsonl` contain NONE of a seeded exchange's secrets, while the
/// UNREDACTED copies DO carry them (so the redaction switch is proven real, not
/// a no-op). A seeded exchange carries secrets in its HEADERS (whole-value
/// header-name redaction) and in its INLINE BODY (value-pattern redaction,
/// including a `Bearer ` token in the body to prove the body matcher catches
/// it). Finally, the shell-quoting routine round-trips hostile bytes to exactly
/// one argument for both shells (structural, never executed).
#[tokio::test(flavor = "multi_thread")]
async fn redaction_golden() {
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path().join("proj");
    let ca_dir = tmp.path().join("ca");
    let upstream = spawn_upstream().await;

    // Unique high-entropy sentinels. No sentinel is a substring of another, none
    // collides with the `[REDACTED]` placeholder, and none contains a token
    // delimiter (space, tab, `;`, `,`, `&`, quote, CR or LF) that would truncate
    // a value-pattern match before the sentinel bytes.
    let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJaUTd4U0VOVEpXVCJ9.c2lnWlE3eFNFTlRKV1RzaWduYXR1cmV4";
    let cookie_secret = "SENTxCOOKIExZq7xK2mNp9vB3";
    let api_key = "SENTxKEYxRt4wV8bXc1dH6jL";
    let body_at = "SENTxATxHy6nL3dQe5fU8wA2";
    let body_ak = "SENTxAKxJu2sF7gWz0kP4yN9";
    let body_bearer = "SENTxBODYxBEARERxMk9pT1rYx4qD";
    let sentinels = [jwt, cookie_secret, api_key, body_at, body_ak, body_bearer];

    // Headers carry secrets caught by header-NAME redaction (Authorization,
    // Cookie, X-API-Key); the inline body carries secrets caught by VALUE
    // patterns (`access_token=`, `api_key=`, and a `Bearer ` token — the
    // carry-forward from Task 0's review: prove the body matcher catches a
    // Bearer/JWT in a body, not only the two `=` patterns).
    let body = format!("access_token={body_at}&api_key={body_ak}&auth=Bearer {body_bearer}");
    let req = format!(
        "POST /submit HTTP/1.1\r\n\
         Host: 127.0.0.1\r\n\
         Authorization: Bearer {jwt}\r\n\
         Cookie: session={cookie_secret}\r\n\
         X-API-Key: {api_key}\r\n\
         Content-Type: application/x-www-form-urlencoded\r\n\
         Content-Length: {len}\r\n\
         Connection: close\r\n\
         \r\n\
         {body}",
        len = body.len(),
    );

    // A fresh ca-dir + --allow the upstream mirror audit_completeness.
    let allow = format!("127.0.0.1:{}", upstream.port());
    let (mut child, cap) = spawn_serve_capturing(
        &project,
        &ca_dir,
        &["--allow", &allow, "--accept-invalid-cert", "127.0.0.1"],
    );
    let api = cap.startup.api;
    let cookie = api_session(api, &token_from_bootstrap_line(&cap.startup.bootstrap_line)).await;
    let admin = [
        ("Cookie", cookie.as_str()),
        ("Sec-Fetch-Site", "same-origin"),
        ("Content-Type", "application/json"),
    ];

    // Seed the exchange by relaying the crafted request through the repeater
    // (raw mode sends and records the bytes verbatim); the response carries the
    // recorded exchange id directly, so no history walk is needed.
    let raw_b64 = base64::engine::general_purpose::STANDARD.encode(req.as_bytes());
    let send = format!(
        "{{\"host\":\"127.0.0.1\",\"port\":{},\"tls\":false,\"raw\":\"{raw_b64}\",\"allow_out_of_scope\":true}}",
        upstream.port()
    );
    let (st, _, sbody) = http(
        api,
        "POST",
        "/api/v1/repeater/send",
        &admin,
        Some(send.as_bytes()),
    )
    .await;
    assert_eq!(
        st,
        200,
        "repeater send: {}",
        String::from_utf8_lossy(&sbody)
    );
    let v: serde_json::Value = serde_json::from_slice(&sbody).unwrap();
    let id = v["exchange_id"]
        .as_str()
        .expect("an exchange id")
        .to_string();

    let read = [
        ("Cookie", cookie.as_str()),
        ("Sec-Fetch-Site", "same-origin"),
    ];

    // The exchange sink is a bounded async queue (committed on a later batch),
    // so the recorded exchange may not be queryable the instant `send` returns.
    // Poll the copy route until the exchange has committed.
    let curl_path = format!("/api/v1/exchanges/{id}/curl?shell=sh");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let (st, _, _) = http(api, "GET", &curl_path, &read, None).await;
        if st == 200 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the seeded exchange {id} never became queryable (last status {st})"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // The REDACTED copies carry NONE of the sentinels: curl (sh), curl (pwsh)
    // and raw.
    for path in [
        format!("/api/v1/exchanges/{id}/curl?shell=sh&redacted=true"),
        format!("/api/v1/exchanges/{id}/curl?shell=pwsh&redacted=true"),
        format!("/api/v1/exchanges/{id}/raw?redacted=true"),
    ] {
        let (st, _, out) = http(api, "GET", &path, &read, None).await;
        assert_eq!(st, 200, "{path}");
        assert_no_sentinels(&out, &sentinels, &format!("redacted {path}"));
    }

    // The UNREDACTED copies DO carry EVERY sentinel: the switch is real, not a
    // no-op that happens to drop bytes either way.
    for path in [
        format!("/api/v1/exchanges/{id}/curl?shell=sh"),
        format!("/api/v1/exchanges/{id}/raw"),
    ] {
        let (st, _, out) = http(api, "GET", &path, &read, None).await;
        assert_eq!(st, 200, "{path}");
        let text = String::from_utf8_lossy(&out);
        for s in sentinels {
            assert!(
                text.contains(s),
                "the UNREDACTED copy {path} should carry the sentinel {s} \
                 (if not, the fixture or the copy route is wrong, not redaction)"
            );
        }
    }

    // Clean shutdown flushes `listener.stop` and closes the captured pipes.
    sigint_and_wait(&mut child.0);
    let output = cap.finish();

    // §11.4: the logs never contain captured bytes.
    assert_no_sentinels(output.as_bytes(), &sentinels, "process stdout+stderr");

    // The audit log records targets (exchange ids) and variant tags, never the
    // captured bytes.
    let audit = std::fs::read(project.join("audit.jsonl")).unwrap();
    assert_no_sentinels(&audit, &sentinels, "audit.jsonl");

    // The shell-quoting routine round-trips hostile bytes to exactly those bytes
    // as ONE argument for both shells. Structural: the quoted output is parsed
    // back the way each shell would, never executed.
    let hostile: &[u8] = b"x'y $(id) \x60tick\x60\nz;w";
    assert_eq!(
        posix_unquote(crosswire_tools::quote(hostile, crosswire_tools::Shell::Posix).as_bytes()),
        hostile,
        "POSIX quote must round-trip to one literal argument",
    );
    assert_eq!(
        pwsh_unquote(
            crosswire_tools::quote(hostile, crosswire_tools::Shell::PowerShell).as_bytes()
        ),
        hostile,
        "PowerShell quote must round-trip to one literal argument",
    );
}

/// Fails if any sentinel appears in `haystack` (as UTF-8 text).
fn assert_no_sentinels(haystack: &[u8], sentinels: &[&str], where_: &str) {
    let text = String::from_utf8_lossy(haystack);
    for s in sentinels {
        assert!(!text.contains(s), "the secret {s} leaked into {where_}");
    }
}

/// Parses a POSIX-shell single-quoted word back to its literal bytes, modelling
/// how `sh` resolves the quoting that [`crosswire_tools::quote`] emits: inside
/// `'...'` every byte is literal; a `\` outside quotes escapes the next byte
/// (this is the `'\''` idiom). The input is one word, so there is no splitting.
fn posix_unquote(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    let mut in_single = false;
    while i < s.len() {
        let b = s[i];
        if in_single {
            if b == b'\'' {
                in_single = false;
            } else {
                out.push(b);
            }
            i += 1;
        } else {
            match b {
                b'\'' => {
                    in_single = true;
                    i += 1;
                }
                b'\\' if i + 1 < s.len() => {
                    out.push(s[i + 1]);
                    i += 2;
                }
                _ => {
                    out.push(b);
                    i += 1;
                }
            }
        }
    }
    out
}

/// Parses a PowerShell single-quoted string back to its literal bytes: the whole
/// argument is wrapped in `'...'` and an embedded `'` is doubled (`''`).
fn pwsh_unquote(s: &[u8]) -> Vec<u8> {
    assert!(
        s.len() >= 2 && s[0] == b'\'' && s[s.len() - 1] == b'\'',
        "a PowerShell-quoted argument is wrapped in single quotes"
    );
    let inner = &s[1..s.len() - 1];
    let mut out = Vec::new();
    let mut i = 0;
    while i < inner.len() {
        if inner[i] == b'\'' && inner.get(i + 1) == Some(&b'\'') {
            out.push(b'\'');
            i += 2;
        } else {
            out.push(inner[i]);
            i += 1;
        }
    }
    out
}

/// Captured process output plus the threads draining the pipes.
struct CapturedServe {
    startup: Startup,
    captured: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    joins: Vec<std::thread::JoinHandle<()>>,
}

impl CapturedServe {
    /// Joins the drain threads (the pipes have closed after the child exits) and
    /// returns all captured lines, newline-joined.
    fn finish(self) -> String {
        for h in self.joins {
            let _ = h.join();
        }
        let lines = self.captured.lock().unwrap_or_else(PoisonError::into_inner);
        lines.join("\n")
    }
}

/// Like [`spawn_serve`] but pipes BOTH stdout and stderr and accumulates every
/// line, so the gate can prove the captured logs carry no secret. Startup is
/// detected from the same stdout lines `wait_serve_startup` watches.
fn spawn_serve_capturing(project: &Path, ca_dir: &Path, extra: &[&str]) -> (Reap, CapturedServe) {
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
    let mut child = Command::new(BIN)
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let captured = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let (tx, rx) = mpsc::channel();

    let stdout = child.stdout.take().unwrap();
    let cap = std::sync::Arc::clone(&captured);
    let h_out = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { return };
            cap.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(line.clone());
            let _ = tx.send(line);
        }
    });

    let stderr = child.stderr.take().unwrap();
    let cap = std::sync::Arc::clone(&captured);
    let h_err = std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines() {
            let Ok(line) = line else { return };
            cap.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(line);
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
    (
        Reap(child),
        CapturedServe {
            startup: Startup {
                api: api.unwrap(),
                bootstrap_line: bootstrap.unwrap(),
            },
            captured,
            joins: vec![h_out, h_err],
        },
    )
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
