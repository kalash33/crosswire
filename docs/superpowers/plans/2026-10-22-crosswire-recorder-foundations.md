# Crosswire alpha.1, part 2: recorder foundations Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The four crates the proxy stands on — `crosswire-model` (the frozen `Message`/`Exchange` schema and the `ExchangeSink`), `crosswire-ca` (local CA and SNI leaf issuance), `crosswire-client` (tokio transport with TLS, the destination gate, and an HTTP/1 session over `crosswire-h1`), `crosswire-store` (crash-safe project storage with the stated durability window) — each testable on its own, plus one integration test that fetches a URL through the client and records it in a store.

**Architecture:** `model` is types only (serde, `bytes`). `ca` wraps `rcgen` and caches leaves. `client` owns a `TlsBackend` trait with a rustls implementation, a `Transport` that resolves DNS then applies the destination gate, and a `Session` that drives `crosswire_h1::Parser` under the re-present contract. `store` is SQLite (WAL) for metadata, append-only zstd pack files for bodies ≥ 16 KiB, a single writer OS thread fed by a bounded queue with an on-disk journal spill, and a broadcast event stream with `received`/`committed` sequence ids. Plan 3 (proxy + server) composes them.

**Tech Stack:** Rust stable, `tokio` (rt-multi-thread, net, io-util, time, sync), `rustls` 0.23 + `tokio-rustls` + `webpki-roots` + `rustls-pki-types`, `rcgen` 0.14, `rusqlite` (bundled) 0.32+, `zstd`, `sha2`, `ulid`, `serde`/`serde_json`, `bytes`, `thiserror`, `tracing`, `fs2`/`rustix` for `flock`, `crosswire-core`, `crosswire-h1`.

**Spec:** `docs/superpowers/specs/2026-10-22-crosswire-design.md` (v4): §5 crate table (`model`, `ca`, `client`, `store` rows), §5.2 (`Transport`/`Session`/`Stream`/`WritePlan`/`TlsBackend`), §6.0–6.1 (what is recorded; the frozen `Message` schema; fidelity labels), §8 (events with sequence ids and `lagged`), §9 (storage layout, packs, write path, durability sentence, audit log), §11.2 (destination gate), §11.3 (CA handling), §11.6 (`Limits` rows used here), §11.9 (upstream TLS policy), §13 (kill -9 test), §14 alpha.1 mini-DoD.

## Global Constraints

- All new crates: `#![forbid(unsafe_code)]`, `#![deny(missing_docs)]` via workspace lints; public enums `#[non_exhaustive]`; private fields with accessors unless the plan says `pub`; `publish = false` for now; every crate has a `//!` doc.
- `crosswire-model` depends on `crosswire-core`, `crosswire-h1`, `bytes`, `serde`, `ulid` only; `std` feature default; must build with `--no-default-features` (alloc) — JSON helpers live behind `std`.
- `crosswire-ca` depends on `rcgen`, `rustls-pki-types`, `time`, `sha2`; no network; no trust installation (that is the server's job).
- `crosswire-client` depends on `tokio`, `rustls`, `tokio-rustls`, `webpki-roots`, `crosswire-h1`, `crosswire-core`, `crosswire-model`; it never depends on `store`.
- `crosswire-store` depends on `rusqlite` (bundled), `zstd`, `sha2`, `ulid`, `serde_json`, `crosswire-model`; it exposes an `ExchangeSink` and never depends on `client` or `proxy`.
- Frozen schema (§6.1): `Message { kind, fidelity, head, framing, diagnostics, body, diffs, connection }`, `Exchange { id: ULID, request, response?, timing, scope_state, tags, notes, namespace, state }`, `Marker { id, kind, from_ulid?, to_ulid?, detail }`; evolution additive-only; unknown fields ignored on read (`#[serde(default)]`, never `deny_unknown_fields`); `schema_version` = 1; SQLite `user_version` = 1.
- Durability sentence (§9.3, binding): anything whose `committed` event you saw survives a process crash; a crash loses at most one commit batch (≤ 250 ms / 1,000 exchanges) when the writer keeps up and at most the bounded queue (64 MiB / 10k) otherwise; power loss loses at most one checkpoint interval (30 s / 64 MiB); every loss is marked by a gap row written by the startup sweep.
- `Limits` used here (§11.6): `inline_sqlite` 16 KiB; store queue 64 MiB / 10k; journal disk cap 1 GiB; `audit.jsonl` cap 64 MiB; leaf-cert LRU 4k / mint rate 50 per s; TLS handshake timeout 15 s; DNS timeout 5 s / concurrency 64; zstd decompression size 64 MiB / ratio 100:1.
- Destination gate (§11.2): refuse loopback, link-local, and cloud-metadata ranges (169.254.169.254, fd00:ec2::254, 100.100.100.200) AFTER DNS resolution regardless of spelling, unless the per-project allowlist names `host:port`; the server's own listener ports are refused even when loopback is allowlisted (listener ports are a parameter of the gate; Plan 3 supplies them).
- Upstream TLS (§11.9): verify chains against `webpki-roots` by default; "accept invalid" only per host; record chain result, version, cipher and ALPN on every connection; TLS 1.2 minimum.
- Commit messages end with the two attribution lines from the session reminder. Hours are logged in `docs/log.md` as `n/m` (not measured) until the maintainer logs real hours.

## Review Focus

1. A body of exactly 16 KiB must be stored inline and 16 KiB + 1 in a pack; both must read back byte-identical (Task 6 `inline_boundary` test).
2. A pack file torn mid-record (power loss) must be truncated by the startup sweep and the exchange that referenced it marked with a gap, with no panic and `integrity_check` ok (Task 6 `torn_tail_recovery`).
3. Killing the process with SIGKILL while 5,000 exchanges are in flight must lose no exchange whose `committed` event was observed, must never corrupt the DB, and must write one gap marker covering the rest (Task 7 `kill_minus_nine`).
4. A hostname that resolves to a loopback or metadata address (`localhost`, `127.1`, `0x7f000001`, `[::1]`, `169.254.169.254`) must be refused by the gate after resolution, and the same host:port allowlisted must pass (Task 3 `gate_refuses_after_resolution`).
5. A response with `Transfer-Encoding: chunked` whose chunk extension is split across two TCP reads must be recorded with the correct extension bytes — the re-present contract honoured by `Session` (Task 4 `split_extension_across_reads`).

---

### Task 0: Workspace additions

**Files:**
- Modify: `Cargo.toml` (workspace deps table), `justfile` (`features` recipe covers the new crates), `.github/workflows/ci.yml` (nothing new needed if `just features` is used; verify), `deny.toml` (add licences only if `cargo deny check` names them).
- Create: `crates/crosswire-model/{Cargo.toml,src/lib.rs}`, `crates/crosswire-ca/{Cargo.toml,src/lib.rs}`, `crates/crosswire-client/{Cargo.toml,src/lib.rs}`, `crates/crosswire-store/{Cargo.toml,src/lib.rs}`, `crates/crosswire-recorder-tests/{Cargo.toml,src/lib.rs}` (integration crate, `publish = false`, dev-deps on client + store + ca).

**Interfaces:**
- Produces: four library crates with the dependency sets from Global Constraints, pinned in `[workspace.dependencies]`: `tokio = { version = "1", default-features = false, features = ["rt-multi-thread","net","io-util","time","sync","macros"] }`, `rustls = { version = "0.23", default-features = false, features = ["ring","std","tls12"] }`, `tokio-rustls = "0.26"`, `webpki-roots = "0.26"`, `rustls-pki-types = "1"`, `rcgen = { version = "0.14", features = ["pem","x509-parser"] }`, `rusqlite = { version = "0.32", features = ["bundled"] }`, `zstd = "0.13"`, `sha2 = "0.10"`, `ulid = "1"`, `serde = { version = "1", default-features = false, features = ["derive","alloc"] }`, `serde_json = "1"`, `thiserror = "2"`, `tracing = "0.1"`, `time = "0.3"`, `rustix = { version = "0.38", features = ["fs"] }` (for `flock`).

- [ ] **Step 1: Add the crates and workspace dependencies**; each `src/lib.rs` has the crate doc, `#![cfg_attr(not(feature = "std"), no_std)]` only for `crosswire-model`, and nothing else.
- [ ] **Step 2: Verify** `cargo build --workspace --locked`, `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`, `cargo deny check`, and `cargo check -p crosswire-model --locked --no-default-features`.
- [ ] **Step 3: Commit** `chore: scaffold model, ca, client, store and recorder-tests crates`.

### Task 1: `crosswire-model` — the frozen schema and the sink

**Files:**
- Create: `crates/crosswire-model/src/{fidelity.rs,message.rs,exchange.rs,marker.rs,event.rs,sink.rs,redact.rs}`; modify `src/lib.rs`.
- Test: `crates/crosswire-model/tests/{schema_roundtrip.rs,unknown_fields.rs,redaction.rs}`

**Interfaces:**
- Produces (all `serde` `Serialize + Deserialize`, `Debug, Clone, PartialEq`; enums `#[non_exhaustive]`; `#[serde(rename_all = "kebab-case")]` on enums):
  - `pub const SCHEMA_VERSION: u32 = 1;`
  - `pub enum Fidelity { Raw, DecodedOrdered, Normalized, Opaque, Generated }`
  - `pub enum MessageKind { Http1Raw, Http2Frames, Normalized, WsFrames, Tunnel }`
  - `pub enum Head { Http1Raw { bytes: Bytes, spans: HeadSpans, parser_version: u32 }, Normalized { map: Vec<(String, Vec<u8>)> }, Tunnel { bytes_in: u64, bytes_out: u64, duration_ms: u64, reason: TunnelReason } }` where `HeadSpans` is a serde-able copy of `crosswire_h1::HeadView` (start line spans, header spans, framing) with `impl From<&HeadView>`; `pub enum TunnelReason { Ws, SniPassthrough, AutoPassthrough, Undelimitable }`; `Bytes` serialised as base64 under `std` (use `serde_bytes`-style manual impl) — document.
  - `pub enum Body { None, Inline { bytes: Bytes }, Blob { hash: [u8; 32], len: u64, truncated_at: Option<u64>, capture_dropped: bool } }`
  - `pub struct Diff { op: ChangeOp, span: Span, before: Bytes, after: Bytes, reason: ChangeReason }` (re-exports from core)
  - `pub struct ConnectionInfo { target_addr: String, port: u16, client_addr: Option<String>, sni: Option<String>, alpn: Option<String>, tls_version: Option<String>, cipher: Option<String>, upstream_chain_result: ChainResult, cert_override: bool, conn_id: u64, reused: bool }`; `pub enum ChainResult { NotTls, Valid, Invalid { reason: String }, Overridden }`
  - `pub struct Message { kind, fidelity, head, framing: FramingRecord, diagnostics: Vec<Diagnostic>, body: Body, diffs: Vec<Diff>, connection: ConnectionInfo }` with `Message::http1_raw(head: &RawMessage, body: Body, connection: ConnectionInfo) -> Message` (fills `framing` from the `HeadView`, `diagnostics` from the message, `fidelity = Raw`); `FramingRecord` = serde-able mirror of `BodyFraming` + rule name.
  - `pub struct Timing { connect_ms: u32, tls_ms: u32, ttfb_ms: u32, total_ms: u32 }`; `pub enum ScopeState { In, Out, Empty, HostMismatch }`; `pub struct Namespace { kind: NamespaceKind, owner: Option<Ulid> }`, `pub enum NamespaceKind { History, Repeater, Intruder }`; `pub enum ExchangeState { Received, Committed, Error }`
  - `pub struct Exchange { id: Ulid, request: Message, response: Option<Message>, timing, scope_state, tags: Vec<String>, notes: String, namespace, state }` with `Exchange::new(request: Message) -> Exchange` (fresh ULID, `Received`, `History`, `Empty`).
  - `pub struct Marker { id: Ulid, kind: MarkerKind, from_ulid: Option<Ulid>, to_ulid: Option<Ulid>, detail: String }`; `pub enum MarkerKind { Gap, LimitExceeded, Error }`
  - `pub enum Event { Received { id: Ulid, seq: u64 }, Committed { ids: Vec<Ulid>, seq: u64 }, Marker { marker: Marker, seq: u64 }, Error { detail: String, seq: u64 }, LimitExceeded { detail: String, seq: u64 }, Lagged { missed: u64, seq: u64 } }` with `Event::seq(&self) -> u64`.
  - `pub trait ExchangeSink: Send + Sync { fn submit(&self, exchange: Exchange) -> Result<(), SinkError>; }`; `pub enum SinkError { Full, Closed }`.
  - `pub struct RedactionPolicy { header_names: Vec<String>, value_patterns: Vec<String> }` with `RedactionPolicy::default_policy()` listing Authorization, Proxy-Authorization, Cookie, Set-Cookie, X-API-Key, X-Auth-Token, X-Amz-Security-Token and patterns `Bearer `, `access_token=`, `api_key=`, a JWT shape; `fn redact_message(&self, m: &Message) -> Message` (replaces matched header values and matched substrings with `[REDACTED]`; head bytes are rebuilt via `crosswire_h1::write_exact` after span-based replacement — document that the result is `Fidelity::Normalized`).

- [ ] **Step 1: Write the failing tests**

```rust
// tests/schema_roundtrip.rs
#[test] fn exchange_roundtrips_json() {
    let raw = parse_req(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n");
    let ex = Exchange::new(Message::http1_raw(&raw, Body::None, conn("x", 80)));
    let s = serde_json::to_string(&ex).unwrap();
    let back: Exchange = serde_json::from_str(&s).unwrap();
    assert_eq!(back, ex);
    assert!(s.contains("\"fidelity\":\"raw\"") && s.contains("\"kind\":\"http1-raw\""));
}
// tests/unknown_fields.rs
#[test] fn unknown_fields_are_ignored() {
    let mut v: serde_json::Value = serde_json::to_value(sample_exchange()).unwrap();
    v["future_field"] = serde_json::json!(42);
    v["request"]["future_nested"] = serde_json::json!({"a": 1});
    let back: Exchange = serde_json::from_value(v).unwrap();
    assert_eq!(back.id(), sample_exchange().id());
}
// tests/redaction.rs
#[test] fn default_policy_redacts_cookie_and_bearer() {
    let raw = parse_req(b"GET / HTTP/1.1\r\nHost: x\r\nCookie: a=1\r\nX-T: Bearer abc.def.ghi\r\n\r\n");
    let m = Message::http1_raw(&raw, Body::None, conn("x", 80));
    let r = RedactionPolicy::default_policy().redact_message(&m);
    let Head::Http1Raw { bytes, .. } = r.head() else { panic!() };
    assert!(!bytes.windows(3).any(|w| w == b"a=1")); assert!(!bytes.windows(7).any(|w| w == b"abc.def"));
    assert_eq!(r.fidelity(), Fidelity::Normalized);
}
```

- [ ] **Step 2: Run** `cargo test -p crosswire-model --locked` — expected: compile errors.
- [ ] **Step 3: Implement** the types; `Bytes` ser/de via a small `mod b64` under `std` (base64 without a new dependency: write the 20-line encoder/decoder, or use `serde_json`'s byte array when not `std` — choose base64 under `std`, `Vec<u8>` array otherwise, and document).
- [ ] **Step 4: Run** the tests and `cargo check -p crosswire-model --locked --no-default-features` — expected: PASS.
- [ ] **Step 5: Commit** `feat(model): frozen Message/Exchange schema, events, sink, redaction policy`.

### Task 2: `crosswire-ca` — CA and leaf issuance

**Files:**
- Create: `crates/crosswire-ca/src/{ca.rs,issuer.rs,keystore.rs,error.rs}`; modify `src/lib.rs`.
- Test: `crates/crosswire-ca/tests/{issue.rs,keystore.rs}`

**Interfaces:**
- Produces:
  - `pub struct Ca { /* rcgen::Certificate + KeyPair */ }` with `Ca::generate(subject_suffix: &str) -> Result<Ca, CaError>` (CN = `Crosswire CA <suffix>`, `basicConstraints CA:TRUE`, `keyUsage keyCertSign|cRLSign`, validity 10 years, notBefore −1 day, P-256), `Ca::from_pem(cert_pem: &str, key_pem: &str) -> Result<Ca, CaError>`, `cert_pem()`, `cert_der() -> Vec<u8>`, `key_pem()` (only used by the keystore), `spki_sha256_base64() -> String` (for Chromium's `--ignore-certificate-errors-spki-list`).
  - `pub trait KeyStore { fn load(&self) -> Result<Option<(String, String)>, CaError>; fn save(&self, cert_pem: &str, key_pem: &str) -> Result<(), CaError>; }`; `pub struct FileKeyStore { dir: PathBuf }` writing `ca.crt` (0644) and `ca.key` (0600, directory 0700; on Windows no mode bits); `Ca::load_or_generate(store: &dyn KeyStore, subject_suffix) -> Result<Ca, CaError>`.
  - `pub struct LeafIssuer { /* Ca, LRU<HostKey, Arc<rustls::sign::CertifiedKey>>, one leaf KeyPair per issuer, rate limiter */ }` with `LeafIssuer::new(ca: Arc<Ca>, limits: IssuerLimits) -> LeafIssuer`, `IssuerLimits { cache_entries: 4096, mint_rate_per_sec: 50 }` (`Default`), `fn issue(&self, host: &ServerName) -> Result<Arc<CertifiedKey>, CaError>` (SAN DNS for names, IP SAN for literals, EKU serverAuth, notAfter = min(398 days, CA notAfter − 1 day), notBefore −1 day, cached by host, `CaError::RateLimited` when the mint rate is exceeded), `fn cached_len(&self) -> usize`.
  - `#[non_exhaustive] pub enum CaError { Generate(String), Pem(String), Io(std::io::Error), RateLimited, InvalidHost }`.

- [ ] **Step 1: Write the failing tests**

```rust
// tests/issue.rs
#[test] fn leaf_validates_against_ca_with_webpki() {
    let ca = Ca::generate("test").unwrap();
    let issuer = LeafIssuer::new(Arc::new(ca.clone()), IssuerLimits::default());
    let ck = issuer.issue(&ServerName::try_from("example.test").unwrap()).unwrap();
    // build a rustls RootCertStore from ca.cert_der(), verify ck.cert[0] for "example.test" with rustls::server::WebPkiClientVerifier-equivalent server verification (use rustls::client::WebPkiServerVerifier::builder)
    assert_verifies(&ca, &ck, "example.test");
    assert!(!assert_verifies_ok(&ca, &ck, "other.test"));
}
#[test] fn ip_literal_gets_ip_san() { /* issue for ServerName::IpAddress(127.0.0.1) and parse the SAN with x509-parser */ }
#[test] fn cache_hit_returns_same_arc() { /* two issues for the same host → Arc::ptr_eq */ }
#[test] fn mint_rate_limit() { /* limits.mint_rate_per_sec = 2; third distinct host within a second → CaError::RateLimited */ }
#[test] fn leaf_validity_bounds() { /* notAfter ≤ now + 398 days and < CA notAfter; notBefore ≤ now − 23h */ }
// tests/keystore.rs
#[test] fn file_keystore_roundtrip_and_modes() { /* tempdir; load_or_generate twice yields the same cert; on unix key file mode == 0o600, dir 0o700 */ }
```

- [ ] **Step 2: Run** — expected: compile errors.
- [ ] **Step 3: Implement** with `rcgen`; the LRU is a `Mutex<lru-style map>` (write a small ordered map with `VecDeque` + `HashMap`, or add the `lru` crate — choose the `lru` crate, add to workspace deps); the rate limiter is a token bucket on `Instant`.
- [ ] **Step 4: Run** — expected: PASS.
- [ ] **Step 5: Commit** `feat(ca): local CA, file keystore, cached SNI leaf issuance`.

### Task 3: `crosswire-client` — `TlsBackend`, `Transport`, destination gate

**Files:**
- Create: `crates/crosswire-client/src/{tls.rs,rustls_backend.rs,gate.rs,transport.rs,error.rs}`; modify `src/lib.rs`.
- Test: `crates/crosswire-client/tests/{gate.rs,transport_tls.rs}`, helper `tests/common/mod.rs` (in-process TLS server using `tokio-rustls` with a `crosswire-ca`-issued leaf, and one with a self-signed cert from a different CA).

**Interfaces:**
- Produces:
  - `pub struct TlsConnectConfig { sni: Option<ServerName<'static>>, alpn: Vec<Vec<u8>>, verify: VerifyMode, client_cert: Option<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>)> }`; `#[non_exhaustive] pub enum VerifyMode { WebPki, AcceptInvalid }`.
  - `pub struct TlsInfo { version: String, cipher: String, alpn: Option<String>, chain_result: ChainResult }` (uses `crosswire_model::ChainResult`).
  - `pub trait TlsBackend: Send + Sync { fn connect<'a>(&'a self, tcp: TcpStream, cfg: TlsConnectConfig) -> Pin<Box<dyn Future<Output = Result<(Box<dyn Conn>, TlsInfo), ClientError>> + Send + 'a>>; }` where `pub trait Conn: AsyncRead + AsyncWrite + Unpin + Send {}` with a blanket impl.
  - `pub struct RustlsBackend { roots: Arc<RootCertStore> }` with `RustlsBackend::webpki_roots()` and `RustlsBackend::with_extra_roots(ders)`; `AcceptInvalid` uses a custom `ServerCertVerifier` that records the failure reason into `ChainResult::Overridden` instead of failing; TLS 1.2 minimum; handshake timeout 15 s → `ClientError::Timeout`.
  - `pub struct DestinationGate { allow: HashSet<(String, u16)>, own_ports: Vec<u16> }` with `DestinationGate::new(own_ports)`, `allow(host: &str, port: u16)`, `fn check(&self, requested_host: &str, port: u16, resolved: &[IpAddr]) -> Result<(), GateRefusal>`; `#[non_exhaustive] pub enum GateRefusal { Loopback(IpAddr), LinkLocal(IpAddr), Metadata(IpAddr), OwnListener(u16) }` — rule: every resolved address is checked; loopback/link-local/metadata refused unless `(host, port)` is allowlisted; `own_ports` refused for loopback addresses even when allowlisted.
  - `pub struct Target { host: String, port: u16, tls: Option<TlsConnectConfig> }`.
  - `pub struct Transport { backend: Arc<dyn TlsBackend>, gate: Arc<DestinationGate>, dns_timeout: Duration /* 5 s */, dns_semaphore: Arc<Semaphore> /* 64 */ }` with `Transport::new(backend, gate)`, `async fn connect(&self, target: &Target) -> Result<Connection, ClientError>`; `pub struct Connection { io: Box<dyn Conn>, info: ConnectionInfo /* from model; target_addr = resolved ip, port, sni, alpn, tls_version, cipher, chain_result, conn_id, reused=false */, connect_ms: u32, tls_ms: u32 }` with accessors and `into_parts()`.
  - `#[non_exhaustive] pub enum ClientError { Dns(String), Refused(GateRefusal), Io(std::io::Error), Tls(String), Timeout, Protocol(String) }`.

- [ ] **Step 1: Write the failing tests**

```rust
// tests/gate.rs
#[test] fn gate_refuses_after_resolution() {
    let g = DestinationGate::new(vec![8080]);
    for ip in ["127.0.0.1", "::1", "169.254.169.254", "fd00:ec2::254", "100.100.100.200", "169.254.1.1"] {
        assert!(g.check("whatever.example", 443, &[ip.parse().unwrap()]).is_err(), "{ip}");
    }
    assert!(g.check("example.com", 443, &["93.184.216.34".parse().unwrap()]).is_ok());
    let mut g = DestinationGate::new(vec![8080]); g.allow("localhost", 3000);
    assert!(g.check("localhost", 3000, &["127.0.0.1".parse().unwrap()]).is_ok());
    assert!(matches!(g.check("localhost", 8080, &["127.0.0.1".parse().unwrap()]), Err(GateRefusal::OwnListener(8080))));
    assert!(g.check("localhost", 3001, &["127.0.0.1".parse().unwrap()]).is_err()); // allowlist is per host:port
}
// tests/transport_tls.rs (tokio tests)
#[tokio::test] async fn verifies_ca_issued_leaf_and_refuses_unknown_chain() {
    let (ca, addr) = common::spawn_tls_echo_with_ca().await;           // leaf from crosswire-ca
    let t = Transport::new(Arc::new(RustlsBackend::with_extra_roots(vec![ca.cert_der()])), gate_allowing(addr));
    let c = t.connect(&Target::tls(addr, "example.test", VerifyMode::WebPki)).await.unwrap();
    assert!(matches!(c.info().upstream_chain_result, ChainResult::Valid));
    let (_, bad_addr) = common::spawn_tls_echo_self_signed().await;
    let err = t.connect(&Target::tls(bad_addr, "bad.test", VerifyMode::WebPki)).await.unwrap_err();
    assert!(matches!(err, ClientError::Tls(_)));
    let c = t.connect(&Target::tls(bad_addr, "bad.test", VerifyMode::AcceptInvalid)).await.unwrap();
    assert!(matches!(c.info().upstream_chain_result, ChainResult::Overridden));
}
#[tokio::test] async fn records_alpn_version_cipher() { /* server offers h2+http/1.1, client offers http/1.1 → alpn Some("http/1.1"), tls_version "TLSv1_3", cipher non-empty */ }
#[tokio::test] async fn plain_tcp_connection_has_not_tls() { /* Target::plain → ChainResult::NotTls, tls_ms == 0 */ }
#[tokio::test] async fn gate_applies_to_hostnames() { /* Target "localhost" without allowlist → ClientError::Refused(Loopback) */ }
```

- [ ] **Step 2: Run** — expected: compile errors.
- [ ] **Step 3: Implement.** DNS via `tokio::net::lookup_host` under the semaphore and timeout; gate check on the resolved set; connect to the first address that accepts (happy-eyeballs not required); TLS via `tokio_rustls::TlsConnector` with a per-call `ClientConfig` built from `VerifyMode`; extract `TlsInfo` from `get_ref().1` after the handshake.
- [ ] **Step 4: Run** — expected: PASS.
- [ ] **Step 5: Commit** `feat(client): TlsBackend with rustls, destination gate, Transport`.

### Task 4: `crosswire-client` — `Session` over HTTP/1

**Files:**
- Create: `crates/crosswire-client/src/{session.rs,stream.rs,write_plan.rs}`; modify `src/lib.rs`.
- Test: `crates/crosswire-client/tests/session_h1.rs`, helper additions in `tests/common/mod.rs` (an in-process plain HTTP/1 server that returns scripted raw bytes, optionally split at given offsets with small delays, so chunk extensions and trailers can be split across TCP reads).

**Interfaces:**
- Produces:
  - `pub struct WritePlan { segments: Vec<Segment> }`, `pub struct Segment { bytes: Bytes, hold: Option<HoldPoint> }`, `#[non_exhaustive] pub enum HoldPoint { Manual }`; `WritePlan::raw(bytes: Bytes) -> WritePlan`; a plan with any `hold` returns `ClientError::Protocol("holds are not supported yet")` (reserved for v0.3).
  - `pub struct Session { conn: Connection, parser_cfg: ParserConfig, pending: BytesMut /* unconsumed carry */, exchanges: u64 }` with `Session::new(conn: Connection, cfg: ParserConfig)`, `async fn send(&mut self, plan: WritePlan, ctx: ResponseContext) -> Result<ResponseStream<'_>, ClientError>` (writes all segments, records `request_bytes` = exactly the bytes written), `fn info(&self) -> &ConnectionInfo` (`reused` = `exchanges > 0`).
  - `pub struct ResponseStream<'a>` with `async fn head(&mut self) -> Result<RawMessage, ClientError>` (reads until `Progress::Head`, honouring the re-present contract: keeps `pending`, re-presents on `NeedMore`, handles `Step { consumed: 0, .. }`), `async fn next_body(&mut self) -> Result<Option<BodyPiece>, ClientError>` where `pub struct BodyPiece { raw: Bytes, data: Bytes, ext: Option<Bytes>, trailers: Option<Vec<(Bytes, Bytes)>> }` (one piece per `BodyData`; `ChunkExt` and `Trailers` are attached to the following/last piece), `fn ttfb_ms(&self) -> u32`, `fn raw_response(&self) -> &[u8]` (every response byte read so far, in order — the canonical recorded response), `fn body_diagnostics(&self) -> &[Diagnostic]`; on `Done` the session keeps any leftover bytes in `pending` for the next exchange (pipelined responses are not expected; leftover → `ClientError::Protocol`), and for `UntilClose` framing reads to EOF; `Connection: close` semantics: after the response, `Session::is_reusable()` is false if the request or response carried `Connection: close` (case-insensitive) or framing was `UntilClose`.
  - Limits: total response head via `ParseLimits`; `capture_cap` is NOT enforced here (the proxy enforces it); per-read timeout 60 s idle → `ClientError::Timeout`.

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test] async fn content_length_response_roundtrip() {
    let addr = common::spawn_scripted(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello", &[]).await;
    let mut s = Session::new(connect_plain(addr).await, ParserConfig::default());
    let mut r = s.send(WritePlan::raw(Bytes::from_static(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n")), ResponseContext::for_method(b"GET")).await.unwrap();
    let head = r.head().await.unwrap(); assert_eq!(head.head().status_code(head.bytes()), Some(200));
    let mut body = Vec::new(); while let Some(p) = r.next_body().await.unwrap() { body.extend_from_slice(&p.data); }
    assert_eq!(body, b"hello"); assert_eq!(r.raw_response(), b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello");
    assert!(s.is_reusable());
}
#[tokio::test] async fn split_extension_across_reads() {
    let resp = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5;x=yz\r\nhello\r\n0\r\nX-T: 1\r\n\r\n";
    let split_at = resp.len() - 22 + 3;               // inside ";x=yz"
    let addr = common::spawn_scripted(resp, &[split_at, resp.len() - 6]).await;   // also split inside the trailer block
    let mut s = Session::new(connect_plain(addr).await, ParserConfig::default());
    let mut r = s.send(WritePlan::raw(req()), ResponseContext::for_method(b"GET")).await.unwrap();
    r.head().await.unwrap();
    let pieces: Vec<BodyPiece> = collect(&mut r).await;
    assert_eq!(pieces[0].ext.as_deref(), Some(&b"x=yz"[..]));
    assert_eq!(pieces.last().unwrap().trailers.as_ref().unwrap()[0], (Bytes::from_static(b"X-T"), Bytes::from_static(b"1")));
    assert_eq!(r.raw_response(), resp);
}
#[tokio::test] async fn head_context_has_no_body() { /* HEAD → BodyFraming::None, next_body() == None immediately, connection reusable */ }
#[tokio::test] async fn until_close_reads_to_eof_and_is_not_reusable() { /* HTTP/1.0 style response without length; server closes */ }
#[tokio::test] async fn keep_alive_reuse_two_exchanges() { /* two CL responses on one connection; second `info().reused == true` */ }
#[tokio::test] async fn hold_in_plan_is_rejected() { /* Segment with hold → ClientError::Protocol */ }
#[tokio::test] async fn idle_timeout() { /* server never responds; session read timeout set to 200 ms via a builder → ClientError::Timeout */ }
```

- [ ] **Step 2: Run** — expected: compile errors.
- [ ] **Step 3: Implement** the read loop exactly as the h1 README's reference loop: read into `pending`, feed `&pending[..]`, advance by `consumed`, re-present on `NeedMore`, treat `consumed == 0` non-`NeedMore` events as "call again with the same bytes".
- [ ] **Step 4: Run** — expected: PASS.
- [ ] **Step 5: Commit** `feat(client): HTTP/1 Session and ResponseStream over crosswire-h1`.

### Task 5: `crosswire-store` — project layout, schema, locking

**Files:**
- Create: `crates/crosswire-store/src/{project.rs,schema.rs,error.rs,lock.rs}`, `crates/crosswire-store/migrations/0001_init.sql`; modify `src/lib.rs`; create `docs/format.md` (the on-disk format, version 1).
- Test: `crates/crosswire-store/tests/project.rs`

**Interfaces:**
- Produces:
  - `pub struct Project { dir: PathBuf, db: Connection /* writer connection */, lock: ProjectLock }` with `Project::create(dir: &Path) -> Result<Project, StoreError>` (creates `dir` 0700, `project.db`, `packs/`, `journal/`, `audit.jsonl`; `PRAGMA journal_mode=WAL; synchronous=NORMAL; application_id=0x43575052 /* 'CWPR' */; user_version=1`; runs `0001_init.sql`), `Project::open(dir) -> Result<Project, StoreError>` (refuses `user_version > 1` with `StoreError::NewerFormat(v)`, refuses a wrong `application_id`, opens in defensive mode `PRAGMA trusted_schema=OFF`), `Project::open_or_create`, `fn dir(&self) -> &Path`, `fn reader(&self) -> Result<Connection, StoreError>` (a new read-only connection with `busy_timeout` 5 s, `cache_size` and `mmap_size` bounded).
  - `ProjectLock`: `flock` on `project.lock` (exclusive, non-blocking) via `rustix::fs::flock`; `StoreError::Locked` when held; released on drop; the file records the pid and start time (for the startup sweep to know the previous run died).
  - Schema (`0001_init.sql`): `exchanges(ulid TEXT PRIMARY KEY, namespace TEXT, state TEXT, scope_state TEXT, created_at INTEGER, json TEXT NOT NULL /* Exchange minus inline bodies */, req_method TEXT, req_host TEXT, req_path TEXT, resp_status INTEGER, resp_len INTEGER, tags TEXT, notes TEXT)`, `bodies(ulid TEXT, side TEXT CHECK(side IN ('req','resp')), inline BLOB, blob_hash BLOB, len INTEGER, PRIMARY KEY(ulid, side))`, `blobs(hash BLOB PRIMARY KEY, pack_id INTEGER, offset INTEGER, len INTEGER, refcount INTEGER)`, `packs(id INTEGER PRIMARY KEY, path TEXT, committed_len INTEGER)`, `markers(ulid TEXT PRIMARY KEY, kind TEXT, from_ulid TEXT, to_ulid TEXT, detail TEXT)`, `meta(key TEXT PRIMARY KEY, value TEXT)` (`last_committed_ulid`, `clean_shutdown`), indexes on `(created_at)`, `(req_host)`, `(resp_status)`, `(namespace, created_at)`.
  - `#[non_exhaustive] pub enum StoreError { Io(std::io::Error), Sqlite(rusqlite::Error), Locked, NewerFormat(u32), NotAProject, Corrupt(String), QueueFull, Closed }`.

- [ ] **Step 1: Write the failing tests**: `create_then_open_roundtrip` (dirs/files exist with the modes on unix; `user_version == 1`); `second_open_is_locked` (open twice in one process → `Locked`; after dropping the first, opens); `newer_format_refused` (set `user_version=2` by hand → `NewerFormat(2)`); `wrong_application_id_refused`; `reader_is_read_only` (an `INSERT` through `reader()` fails).
- [ ] **Step 2: Run** — expected: compile errors.
- [ ] **Step 3: Implement**; write `docs/format.md` describing the directory, the tables, the pack record layout (Task 6) and the journal (Task 7).
- [ ] **Step 4: Run** — expected: PASS.
- [ ] **Step 5: Commit** `feat(store): project layout, schema v1, locking, docs/format.md`.

### Task 6: `crosswire-store` — bodies, packs, startup sweep

**Files:**
- Create: `crates/crosswire-store/src/{packs.rs,bodies.rs,sweep.rs}`; modify `src/lib.rs`.
- Test: `crates/crosswire-store/tests/packs.rs`

**Interfaces:**
- Produces:
  - Pack record on disk: `magic b"CWPK"` (4) + `len: u32 LE` (compressed length) + `sha256: [u8;32]` (of the RAW bytes) + `zstd frame`; packs are `packs/NNNNNN.pack`, rotated at 256 MiB.
  - `pub struct PackWriter { current: File, id, offset }` with `fn append(&mut self, raw: &[u8]) -> Result<BlobRef, StoreError>` (compress level 3, write, return `BlobRef { hash, pack_id, offset, len }`), `fn fsync(&mut self) -> Result<(), StoreError>`.
  - `pub struct BodyStore` (used by the writer thread under its transaction): `fn put(&mut self, tx: &Transaction, ulid, side, raw: &[u8]) -> Result<BodyLocator, StoreError>` — `raw.len() <= 16 KiB` → `bodies.inline`; else dedupe by sha256 in `blobs` (refcount++) or `PackWriter::append` then insert; `fn get(conn: &Connection, ulid, side) -> Result<Option<Bytes>, StoreError>` (reads inline or pack; verifies the hash on read → `StoreError::Corrupt` on mismatch; refuses decompression beyond 64 MiB or ratio 100:1).
  - `pub fn sweep(project: &mut Project) -> Result<SweepReport, StoreError>`: for each pack, validate records from `packs.committed_len` forward; a torn tail (bad magic / short / hash mismatch) is truncated to the last good record; `blobs` rows pointing past the truncation are deleted and every `bodies` row referencing them has its exchange marked `state='error'` plus a `Marker { kind: Gap, detail: "pack torn" }`; orphan blobs (refcount 0) are deleted; returns counts.

- [ ] **Step 1: Write the failing tests**: `inline_boundary` (16 KiB inline, 16 KiB + 1 in a pack; both read back identical); `dedupe_by_hash` (same 20 KiB body twice → one blob, refcount 2); `torn_tail_recovery` (write 3 bodies, fsync, append a 4th and truncate the pack file 10 bytes into its record, insert its rows as if committed, reopen, `sweep` → pack truncated to the 3rd record end, 4th exchange marked error with a gap marker, `PRAGMA integrity_check == ok`); `hash_mismatch_detected_on_read` (flip a byte in a pack record → `Corrupt`); `decompression_bomb_refused` (a record whose zstd frame expands past the cap → `Corrupt`).
- [ ] **Step 2: Run** — expected: compile errors.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** — expected: PASS.
- [ ] **Step 5: Commit** `feat(store): inline/pack body storage, hash-verified reads, startup sweep`.

### Task 7: `crosswire-store` — writer thread, queue, journal, events, durability

**Files:**
- Create: `crates/crosswire-store/src/{writer.rs,journal.rs,events.rs,audit.rs,sink.rs}`, `crates/crosswire-store/src/bin/killtest-child.rs` (a tiny binary the kill -9 test spawns; `[[bin]] required-features = []`, `publish = false` already), modify `src/lib.rs`.
- Test: `crates/crosswire-store/tests/{writer.rs,durability.rs}`

**Interfaces:**
- Produces:
  - `pub struct Store { project: Arc<Project>, tx: SyncSender<Exchange> /* bounded by count 10_000 and bytes 64 MiB via an AtomicUsize */, events: broadcast::Sender<Event>, writer: JoinHandle<()> }` with `Store::open(project: Project, cfg: StoreConfig) -> Result<Store, StoreError>` (runs `sweep` + journal replay first, writes the `clean_shutdown=false` meta, spawns the writer OS thread), `StoreConfig { batch_interval: 250 ms, batch_max: 1000, queue_max: 10_000, queue_bytes_max: 64 MiB, checkpoint_interval: 30 s, checkpoint_bytes: 64 MiB, journal_cap: 1 GiB }` (`Default`), `fn sink(&self) -> Arc<dyn ExchangeSink>`, `fn subscribe(&self) -> broadcast::Receiver<Event>` (capacity 10k; a lagging receiver gets `Event::Lagged` synthesised by a wrapper `EventStream`), `fn audit(&self, actor: &str, action: &str, target: &str, result: &str) -> Result<(), StoreError>` (append one JSON line to `audit.jsonl`, fsync per row, rotate at 64 MiB), `fn close(self) -> Result<(), StoreError>` (drains the queue, final batch, `clean_shutdown=true`, checkpoint).
  - Writer loop: pull from the queue until `batch_max` or `batch_interval` elapsed; for each exchange: bodies via `BodyStore::put` (packs appended), then `exchanges`/`bodies` rows inside ONE `BEGIN IMMEDIATE` transaction; before `COMMIT`: `PackWriter::fsync`; after `COMMIT`: broadcast `Committed { ids }`; `meta.last_committed_ulid` updated in the same transaction; periodic `PRAGMA wal_checkpoint(PASSIVE)` by interval or WAL size.
  - `ExchangeSink` impl: `submit` emits `Event::Received` and `try_send`s; on a full queue → spill to `journal/<ulid>.json` (one file per exchange, written then `fsync`ed; `StoreError::QueueFull` only if the journal cap is hit, in which case a `Marker { Gap }` is enqueued through a reserved slot and an `Event::Error` broadcast); the writer drains `journal/` live (oldest ULID first) whenever the queue is below a low-water mark (25 %) and fully at startup.
  - Startup sweep addition: if `clean_shutdown != true` at open, insert `Marker { kind: Gap, from_ulid: last_committed_ulid, to_ulid: None, detail: "unclean shutdown" }` and broadcast it.
  - Sequence ids: an `AtomicU64` per store; every `Event` carries the next value.

- [ ] **Step 1: Write the failing tests**

```rust
// tests/writer.rs
#[tokio::test] async fn committed_follows_received_and_is_readable() { /* submit 3; collect events until Committed contains all 3; read back via reader() */ }
#[test] fn batch_boundaries() { /* batch_max 2, interval 10 s: submit 2 → committed quickly; submit 1 → not committed before interval; with interval 50 ms → committed within 200 ms */ }
#[test] fn queue_full_spills_to_journal_and_drains() { /* queue_max 2, pause the writer via a test hook (a Mutex the writer takes per batch); submit 10 → journal has 8 files; unpause → all 10 committed, journal empty */ }
#[test] fn lagged_subscriber_gets_lagged_event() { /* capacity 4, submit 20 without reading, then read → first event is Lagged{missed>0} */ }
#[test] fn audit_rows_are_fsynced_json_lines() { /* two audit() calls → two parseable lines */ }
#[test] fn unclean_shutdown_writes_gap_marker_on_reopen() { /* open, submit, drop without close (mem::forget the Store after joining nothing), reopen → a Gap marker exists */ }
// tests/durability.rs
#[test] fn kill_minus_nine() {
    // spawn `killtest-child <dir> 5000`: it opens the store, submits 5000 exchanges with 20 KiB bodies, prints each Committed id list to stdout as it observes them, never closes.
    // parent: after reading ≥ 1000 committed ids, SIGKILL the child; reopen the project; assert PRAGMA integrity_check == "ok";
    // every id the parent saw as committed exists with state 'committed' and its bodies read back with matching hashes;
    // exactly one Gap marker exists whose from_ulid == last committed id; sweep report has no panic; the pack files validate.
}
```

- [ ] **Step 2: Run** — expected: compile errors.
- [ ] **Step 3: Implement.** The child binary is ~60 lines. Use `std::process::Command` + `libc`-free kill via `child.kill()` (SIGKILL on unix). Make the test `#[cfg(unix)]`.
- [ ] **Step 4: Run** — expected: PASS; run `kill_minus_nine` five times in a loop locally (`for i in 1..5`) and report any flake.
- [ ] **Step 5: Commit** `feat(store): writer thread, bounded queue, journal spill, events, audit, kill -9 durability`.

### Task 8: `crosswire-store` — read API

**Files:**
- Create: `crates/crosswire-store/src/read.rs`; modify `src/lib.rs`.
- Test: `crates/crosswire-store/tests/read.rs`

**Interfaces:**
- Produces: `pub struct ExchangeFilter { method: Option<String>, host_contains: Option<String>, status: Option<RangeInclusive<u16>>, namespace: Option<NamespaceKind>, state: Option<ExchangeState> }`, `pub struct Page<T> { items: Vec<T>, next_cursor: Option<Ulid> }`, `pub struct ExchangeRow { id, created_at, method, host, path, status, resp_len, state, tags }`; on `Store`: `fn list(&self, filter: &ExchangeFilter, after: Option<Ulid>, limit: usize) -> Result<Page<ExchangeRow>, StoreError>` (keyset on ULID descending, parameterised SQL only), `fn get(&self, id: Ulid) -> Result<Option<Exchange>, StoreError>` (rehydrates inline/blob bodies into `Body`), `fn markers(&self, after: Option<Ulid>, limit) -> Result<Page<Marker>, StoreError>`, `fn counts(&self) -> Result<Counts, StoreError>` (`exchanges`, `bytes_inline`, `bytes_packs`, `markers`).

- [ ] **Step 1: Write the failing tests**: `keyset_pagination_is_stable_under_inserts` (insert 25, page by 10 while inserting more → no duplicates, no skips); `filters_use_indexes` (`EXPLAIN QUERY PLAN` for a host filter mentions the index); `get_rehydrates_bodies` (inline and pack bodies both come back as `Body::Inline`/`Body::Blob` with bytes available via a `get_body(id, side)` helper); `sql_injection_in_filter_is_inert` (host_contains = `x' OR 1=1 --` returns 0 rows).
- [ ] **Step 2–5:** RED, implement, GREEN, commit `feat(store): keyset-paginated read API`.

### Task 9: Integration — fetch and record

**Files:**
- Create: `crates/crosswire-recorder-tests/tests/fetch_and_record.rs`, `tests/common/mod.rs` (reuse the client's scripted server helper by copying the ~40 lines; note the duplication in a comment — the proxy plan will move it into a `crosswire-testing` dev crate).

**Interfaces:**
- Consumes: Tasks 2–8. Produces: a documented end-to-end recipe (`record_exchange(session, plan, ctx, sink) -> Ulid`) that Plan 3's relay copies: build the request `Message::http1_raw` from the bytes written, the response from `raw_response()` parsed head + raw body bytes, `Body::Inline`/pack decided by the store, timing from the stream, connection info from `Session::info()`.

- [ ] **Step 1: Write the failing tests**: `fetch_over_tls_and_record` (CA-issued TLS server returns a chunked 40 KiB response → one exchange committed; `get` returns it; `write_exact(req head) == bytes written`; the response body read back from the pack equals the server's raw body bytes; `connection.upstream_chain_result == Valid`, `tls_version` set); `fetch_plain_with_until_close`; `recorded_exchange_survives_reopen` (close the store, reopen, `get` again).
- [ ] **Step 2–5:** RED, implement the helper, GREEN, commit `test(recorder): fetch over TLS and record into a project`.

### Task 10: Docs, ADR-0002, log

**Files:**
- Create: `crates/crosswire-{model,ca,client,store}/README.md`, `docs/adr/0002-store-durability.md` (status Accepted; the durability sentence; why one writer thread + bounded queue + journal spill; why `synchronous=NORMAL` with a stated power-loss window; the rejected alternative of journal-first durability and why — part-time budget; consequences), update `docs/format.md` with the journal file format and the audit line format.
- Modify: `docs/log.md` (rows for Tasks 0–10 as `n/m`), the root `README.md` (what exists now: the four crates and the recorder test; still no proxy/UI), `docs/superpowers/specs/2026-10-22-crosswire-design.md` §17 only if something was deferred from this plan.

- [ ] **Step 1: Write the docs**; every claim checked against the code.
- [ ] **Step 2: Verify** `RUSTDOCFLAGS=-D warnings cargo doc --workspace --no-deps --locked --all-features`, full `cargo test --workspace --locked --all-features`, `cargo deny check`, `cargo package -p crosswire-model --locked --allow-dirty` and `-p crosswire-ca` (both depend only on published-or-local crates — `ca` packages cleanly; `model` depends on `h1` which is unpublished, so expect and document the same limitation as h1).
- [ ] **Step 3: Commit** `docs(recorder): crate READMEs, ADR-0002 store durability, format.md`.

---

## Self-review notes

- **Spec coverage:** §5.2 Transport/Session/Stream/WritePlan/TlsBackend (Tasks 3–4; holds reserved), §6.1 schema (Task 1), §8 events with sequence ids and `lagged` (Task 7), §9.1–9.4 layout/packs/write path/audit (Tasks 5–7), §11.2 destination gate (Task 3), §11.3 CA parameters except OS-keychain and trust installation (Task 2; keychain deferred to the server plan), §11.9 upstream TLS policy (Task 3), §13 kill -9 test (Task 7). **Deferred to Plan 3:** `Handler` trait, SNI passthrough, the proxy listener, `capture_cap`/in-flight budget enforcement, `crosswire serve`, trust installation, keychain feature, retention/purge, FTS, `HeadMethod`-style scope evaluation (scope crate is alpha.2).
- **Type consistency:** `ChainResult`, `ConnectionInfo`, `Exchange`, `Event`, `ExchangeSink`, `SinkError` are defined in Task 1 and used by name in Tasks 3, 4, 7, 9; `RawMessage`/`ResponseContext`/`ParserConfig` come from `crosswire-h1`.
- **Review Focus** items each have a named test in the task that owns the code.
- **Proportion:** tests and signatures only; no transcribed bodies.
