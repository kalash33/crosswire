# Crosswire Proxy and `crosswire serve` Implementation Plan (Plan 3)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Finish 0.1.0-alpha.1, the headless recorder: an HTTP/1.1 MITM proxy that relays a real browser session byte-exactly to upstream and records every exchange into a project directory, started with `crosswire serve`.

**Architecture:** `crosswire-proxy` is a byte pump: one browser connection ↔ one upstream connection, an h1 request parser on the browser side and an h1 response parser on the upstream side, forwarding exactly the bytes each parser consumes while copying them into a bounded capture buffer that the relay never waits on. TLS is terminated per CONNECT with a leaf from `crosswire-ca`; upstream goes through `crosswire-client::Transport` (destination gate, verify-by-default). Recording submits `Exchange`s to the store's `ExchangeSink` from `spawn_blocking`; the proxy never does file I/O. `crosswire-server` is the composition root and the `crosswire` binary.

**Tech Stack:** Rust 1.86 (MSRV), tokio, rustls + tokio-rustls (ring only), clap 4, tracing, existing workspace crates (`crosswire-core/-h1/-model/-ca/-client/-store`).

**Spec:** `docs/superpowers/specs/2026-10-22-crosswire-design.md` — §5 (crate table: proxy 50 h, server 25 h), §5.1 (h1 contract), §6.0 (what is recorded; connection model; framing resolution), §7.1 (proxying; body model; WebSocket; Handler), §8 (events), §9.3 (the proxy never blocks on the store), §11.2 (listener shape, destination gate), §11.3 (CA), §11.6 (Limits), §11.9 (upstream TLS), §12 (`crosswire serve`, CA page), §13 (fixtures, golden), §14 (alpha.1 mini-DoD), §17 (deferred list from Plan 2). ADR-0002 (store durability) and `.superpowers/sdd/2026-10-22-crosswire-recorder-foundations/final-review-report.md` ("Plan 3 must change") are inputs.

## Global Constraints

- `rust-version = "1.86"`; `unsafe_code = "forbid"`; `missing_docs = "deny"` (workspace lints).
- TLS: rustls with `ring` only, `default-features = false`, TLS 1.2 minimum on both sides; `tokio-rustls` likewise (workspace deps).
- The proxy performs no file I/O and never blocks on the store (§9.3): sink submission happens in `spawn_blocking`; a `SinkError` never stalls or closes the relay.
- Canonical request = the bytes written to upstream; canonical response = the bytes read from upstream (§6.0). Browser-side bytes are recoverable from the recorded `Diff` list.
- Limits (§11.6) with these exact defaults: `capture_cap` 50 MiB; `inline_memory` 1 MiB; in-flight body budget 256 MiB; header read 30 s / idle 60 s / total 300 s; TLS handshake 15 s; `tunnel_idle` off; max connections per listener 4,000; pending CONNECTs 256; per-client-IP (non-loopback) 64; SNI ≤ 253 bytes and a valid hostname; handshake rejections 3 per 30 s; pipelining depth 16; `ParseLimits` defaults (64 KiB head / 256 headers / 16 KiB line / 4 KiB chunk-size line / 16 KiB trailers / 1M chunks / 64 body diagnostics).
- Listener shape (§11.2): only absolute-form requests and CONNECT; origin-form with Host = the listener (or `crosswire`) gets the info page; other origin-form is refused. Non-loopback bind requires `--allow-non-loopback` and then proxy authentication is mandatory.
- Upstream TLS (§11.9): verify by default against webpki roots and the CONNECT authority or absolute-form host, never the Host header; on failure, do not relay: generated 502 plus an error. Override only per host (`--accept-invalid-cert HOST`), audited, recorded as `ChainResult::Overridden { reason }` and `cert_override = true`.
- Auto-passthrough on handshake rejection is **off** in alpha.1: scope evaluation ships in alpha.2 and "an empty scope counts as in-scope" (§7.1). Rejections are counted and reported, never silently tunnelled. Manual SNI passthrough works.
- `crosswire-model` wire changes are additive only (`#[serde(default)]` on every new field; new enum variants only on `type`-tagged enums).
- Commits: Conventional Commits, subject ≤ 72 chars, why-body, the user's git identity, **no attribution trailers**.
- Logs never contain captured bytes (§11.4); error text that includes server-supplied names is capped at 1 KiB.

## Review Focus

1. **Host header ≠ CONNECT authority.** A page served from `a.test` makes the browser send `Host: b.test` inside the `a.test` tunnel. Expected: relay to the CONNECT authority, forward the Host header unchanged, `req_host = b.test` (message identity, §4.6). Pinned in Task 5 (`host_header_differs_from_authority_is_forwarded_unchanged`).
2. **Upstream closes mid-body.** A `Content-Length: 100` response that ends after 40 bytes. Expected: the client connection is closed, the exchange is recorded with the 40 bytes, `ExchangeState::Error`, a `body_truncated`-class diagnostic, and the relay task ends (no hang). Pinned in Task 7 (`upstream_close_mid_body_records_error_and_closes_client`).
3. **Slow client, large response.** A client that reads 1 KiB/s while upstream sends 50 MiB. Expected: the relay reads upstream only as fast as the client drains (bounded 64 KiB read buffer), the capture buffer stops at `capture_cap`, process RSS stays flat. Pinned in Task 6 (`slow_client_does_not_buffer_upstream`) and Task 9 (`rss_flat_under_large_until_close_response`).
4. **Client disconnects mid-request-body.** Expected: the upstream connection is closed, an exchange with the partial request and `ExchangeState::Error` is recorded, no panic, counters decrement. Pinned in Task 7 (`client_disconnect_mid_request_body_records_partial_request`).
5. **CONNECT to the proxy's own port or loopback.** Expected: refused by the destination gate after resolution, a generated 502 with the allow hint, an `Error` exchange recorded, the upstream connection never attempted. Pinned in Task 7 (`connect_to_own_listener_is_refused_with_hint`).

---

### Task 0: Scaffold `crosswire-proxy` and `crosswire-server`

**Files:**
- Create: `crates/crosswire-proxy/Cargo.toml`, `crates/crosswire-proxy/src/lib.rs`, `crates/crosswire-server/Cargo.toml`, `crates/crosswire-server/src/main.rs`
- Modify: `Cargo.toml` (workspace deps), `justfile`, `deny.toml` (only if a new licence appears)

**Interfaces:**
- Produces: two crates that compile and pass `just lint`, `just features`, `just doc`; `crosswire` binary name.

- [ ] **Step 1: Add workspace dependencies** to `Cargo.toml [workspace.dependencies]`: `clap = { version = "4", features = ["derive"] }`, `tracing-subscriber = { version = "0.3", default-features = false, features = ["fmt", "env-filter"] }`, `dirs = "6"`, `base64 = "0.22"`. Extend the `tokio` workspace features with `"signal"`.
- [ ] **Step 2: `crosswire-proxy/Cargo.toml`**: name `crosswire-proxy`, version `0.0.0`, `publish = false`, description "HTTP/1.1 MITM proxy engine for Crosswire". Dependencies: `crosswire-core`, `-h1`, `-model`, `-client`, `-ca` (path + `version = "0.0.0"`), `tokio`, `rustls`, `tokio-rustls`, `rustls-pki-types`, `bytes` (as in client), `sha2`, `thiserror`, `tracing`, `base64`. Dev: `tempfile`, `crosswire-store`, `webpki-roots`, `serde_json`, `ulid`. `[lints] workspace = true`. `lib.rs` holds the crate doc and `#![forbid(unsafe_code)]` comes from the workspace lint.
- [ ] **Step 3: `crosswire-server/Cargo.toml`**: name `crosswire-server`, `publish = false`, `[[bin]] name = "crosswire" path = "src/main.rs"`. Dependencies: `crosswire-proxy`, `-store`, `-ca`, `-client`, `-model`, `clap`, `tokio`, `tracing`, `tracing-subscriber`, `dirs`, `thiserror`. Dev: `tempfile`, `rustls`, `tokio-rustls`, `rustls-pki-types`. `main.rs`: `fn main() { println!("crosswire"); }` for now.
- [ ] **Step 4: justfile**: add `serve *ARGS:` → `cargo run --locked -p crosswire-server -- serve {{ARGS}}`; add to `features:` `cargo check --locked -p crosswire-proxy` (no features exist; the line documents that).
- [ ] **Step 5: Verify** `cargo build --workspace --locked`, `just lint`, `just doc`, `cargo deny check` all exit 0.
- [ ] **Step 6: Commit** `chore: scaffold crosswire-proxy and crosswire-server crates`.

---

### Task 1: Model and client additions the proxy depends on

**Files:**
- Modify: `crates/crosswire-model/src/message.rs` (`Body`, `ChainResult`, `Message`, `ConnectionInfo`), `crates/crosswire-model/src/sink.rs`, `crates/crosswire-model/src/lib.rs`
- Modify: `crates/crosswire-client/src/transport.rs:214` (target_addr), `crates/crosswire-client/src/rustls_backend.rs` (Overridden reason), client tests that assert `target_addr` is an IP
- Modify: `crates/crosswire-store/src/writer.rs` (~801-830: carry capture fields; implement `submit_marker`), `crates/crosswire-store/src/sink.rs`, `crates/crosswire-store/src/read.rs` (length from `relay` when present), `docs/format.md`
- Test: `crates/crosswire-model/tests/schema_additive.rs`, `crates/crosswire-store/tests/writer.rs`, `crates/crosswire-client/tests/transport_tls.rs`

**Interfaces:**
- Produces (model):
  - `pub struct Capture { pub len: u64, pub sha256: [u8; 32] }` (serde, `#[serde(with = "bytes_serde::hash")]` on `sha256`) — the relay-path length and hash of the whole body.
  - `Body::Inline { bytes, #[serde(default)] truncated_at: Option<u64>, #[serde(default)] capture_dropped: bool, #[serde(default)] relay: Option<Capture> }` and `Body::Blob { …existing…, #[serde(default)] relay: Option<Capture> }`. Constructor `Body::inline(bytes: Bytes) -> Body` (all new fields default) so existing call sites change to that.
  - `ChainResult::Overridden { #[serde(default)] reason: String }`.
  - `Message::with_interim(self, interim: Vec<Bytes>) -> Message`, `Message::interim(&self) -> &[Bytes]`; field `#[serde(default, with = "bytes_serde::vec")] interim: Vec<Bytes>` (1xx heads forwarded before the final response, raw bytes).
  - `ConnectionInfo::with_peer_addr(self, v: impl Into<String>)`, `peer_addr(&self) -> Option<&str>`; `target_addr` is now the configured host string (§4.6).
  - `ExchangeSink::submit_marker(&self, marker: Marker) -> Result<(), SinkError>` (new required method; the store writes the marker row and emits `Event::Marker`).
- Produces (client): `Transport::connect` builds `ConnectionInfo::new(target.host(), target.port).with_peer_addr(ip.to_string())`; `Overridden { reason }` carries `TlsInfo::chain_error`.

- [ ] **Step 1: Write failing model tests** in `tests/schema_additive.rs`:
  - `old_inline_body_json_still_deserialises`: `{"type":"inline","bytes":"aGk="}` → `Body::Inline` with `truncated_at: None`, `capture_dropped: false`, `relay: None`.
  - `old_overridden_json_still_deserialises`: `{"type":"overridden"}` → `ChainResult::Overridden { reason: "" }`.
  - `message_without_interim_deserialises_empty`.
  - `capture_round_trips` with `len: 70_000`, a fixed hash, `truncated_at: Some(1024)`.
- [ ] **Step 2: Run** `cargo test -p crosswire-model --all-features` → the new tests fail to compile.
- [ ] **Step 3: Implement** the model changes above; update `redact.rs` so `redact_message` preserves the new fields and clears nothing in `interim` beyond what it does for heads (interim heads are redacted with the same header policy: rebuild each via the existing head-redaction path).
- [ ] **Step 4: Store**: in `writer.rs` the Inline→Blob rewrite copies `truncated_at`, `capture_dropped`, `relay`; `ExchangeRow` length columns use `relay.len` when present, else stored length. Implement `submit_marker` on the store sink: enqueue `Msg::Marker(marker)`; the writer inserts it in the next batch (same `markers` table the gap rows use) and emits `Event::Marker`. Test `submit_marker_writes_row_and_event` in `tests/writer.rs`: markers(None, 10) contains it; event seen.
- [ ] **Step 5: Client**: change `transport.rs:214` to the host string plus `with_peer_addr`; fill `Overridden { reason }` from the recorded verifier error. Fix tests in `transport_tls.rs` / `session_h1.rs` that expected an IP in `target_addr` (assert `peer_addr()` instead). Update `crates/crosswire-recorder-tests` to `Body::inline`.
- [ ] **Step 6: `docs/format.md`**: document `relay`, `truncated_at`, `capture_dropped` on bodies, `interim`, `peer_addr`, and that `target_addr` is the configured host; `req_host` fallback text now refers to that host.
- [ ] **Step 7: Verify** `cargo test --workspace --locked --all-features` → 0 failures; `just lint`.
- [ ] **Step 8: Commit** `feat(model): capture fields, interim heads, peer_addr, override reason, markers`.

---

### Task 2: Proxy limits, config, `Handler`, generated responses

**Files:**
- Create: `crates/crosswire-proxy/src/limits.rs`, `src/config.rs`, `src/handler.rs`, `src/generated.rs`, `src/error.rs`
- Test: `crates/crosswire-proxy/tests/generated.rs`, unit tests in `limits.rs`

**Interfaces:**
- Produces:
  - `pub struct ProxyLimits { pub capture_cap: u64, pub inline_memory: u64, pub body_budget: u64, pub header_read: Duration, pub idle: Duration, pub total: Duration, pub tls_handshake: Duration, pub tunnel_idle: Option<Duration>, pub max_connections: usize, pub max_pending_connect: usize, pub max_per_client_ip: usize, pub max_pipelined: usize, pub rejections_per_window: u32, pub rejection_window: Duration, pub parse: ParserConfig }` with `Default` = the Global Constraints values; `#[non_exhaustive]` plus `ProxyLimits::builder()` with one setter per field.
  - `pub struct ProxyConfig { pub bind: SocketAddr, pub allow_non_loopback: bool, pub proxy_auth: Option<ProxyAuth>, pub passthrough: Vec<String>, pub accept_invalid_cert: Vec<String>, pub limits: ProxyLimits }`; `pub struct ProxyAuth { user: String, pass: String }` with `ProxyAuth::new`; `ProxyConfig::validate(&self) -> Result<(), ProxyError>`: non-loopback bind without `allow_non_loopback` → `ProxyError::NonLoopbackBind`; non-loopback with no `proxy_auth` → `ProxyError::AuthRequired`; a passthrough entry that is not a hostname or `.suffix` → `ProxyError::Config(String)`.
  - `pub trait Handler: Send + Sync { fn on_request_head(&self, req: &RawMessage, conn: &ConnectionInfo) -> Decision; fn on_request_body_chunk(&self, data: &[u8]) -> Decision; fn on_response_head(&self, resp: &RawMessage, req: &RawMessage) -> Decision; fn on_response_body_chunk(&self, data: &[u8]) -> Decision; }` with `#[non_exhaustive] pub enum Decision { Forward, Drop }` and `pub struct NoopHandler;` (all `Forward`). `Hold` and `Modify` arrive with intercept in alpha.2 (documented on the enum).
  - `pub(crate) fn generated(status: u16, reason: &str, body: &str, conn: ConnectionInfo) -> (Bytes, Message)`: `HTTP/1.1 {status} {reason}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: N\r\nConnection: close\r\nX-Crosswire-Generated: 1\r\n\r\n{body}`; the `Message` is `Message::http1_raw` of those bytes (parse them with `Parser::response`) then `.with_fidelity(Fidelity::Generated)`.
  - Fixed copy (exact strings live in `generated.rs` as consts): `BAD_GATEWAY_CONNECT = "Crosswire could not connect to {host}:{port}: {reason}"`, `BAD_GATEWAY_TLS = "Crosswire refused the upstream certificate for {host}:{port}: {reason}. Pass --accept-invalid-cert {host} to override for this host (audited)."`, `REFUSED_DESTINATION = "Crosswire refused {host}:{port} ({refusal}). Allow it for this project with: crosswire serve --allow {host}:{port}"`, `GATEWAY_TIMEOUT = "Crosswire timed out waiting for {host}:{port} ({phase})"`, `NOT_A_PROXY_REQUEST = "Crosswire is a proxy. Configure your browser to use it, or open http://crosswire/ for the CA and setup guide."`.
  - `pub enum ProxyError { Config(String), NonLoopbackBind, AuthRequired, Bind(std::io::Error), Io(std::io::Error), Client(ClientError), Tls(String), Protocol(String), Timeout(&'static str), Limit(&'static str) }` (thiserror, `#[non_exhaustive]`).

- [ ] **Step 1: Failing tests**: `limits.rs` unit `defaults_match_spec` (every field equals the Global Constraints value); `tests/generated.rs`: `generated_502_is_parseable_and_marked` (status 502, `Fidelity::Generated`, `Content-Length` equals body length, `Connection: close` present), `config_refuses_non_loopback_without_flag`, `config_requires_auth_on_non_loopback`, `config_rejects_bad_passthrough_entry` (`"not a host!"`).
- [ ] **Step 2: Run** `cargo test -p crosswire-proxy` → fail to compile.
- [ ] **Step 3: Implement** the four modules; `lib.rs` re-exports `ProxyLimits, ProxyConfig, ProxyAuth, Handler, Decision, NoopHandler, ProxyError`.
- [ ] **Step 4: Run** tests → pass; `just lint`.
- [ ] **Step 5: Commit** `feat(proxy): limits, config validation, Handler trait, generated responses`.

---

### Task 3: Listener and front end (CONNECT, absolute-form, info page, proxy auth, caps)

**Files:**
- Create: `crates/crosswire-proxy/src/listener.rs`, `src/front.rs`, `src/info_page.rs`, `src/counters.rs`
- Modify: `src/lib.rs`
- Test: `crates/crosswire-proxy/tests/front.rs`, `tests/common/mod.rs` (shared harness: `ProxyHarness::start(cfg, handler) -> (SocketAddr, ShutdownHandle, Arc<TestSink>)`, raw TCP client helpers `connect_raw`, `send_and_read_until_close`)

**Interfaces:**
- Consumes: `ProxyConfig`, `generated`, `ParserConfig` from `ProxyLimits.parse`.
- Produces:
  - `pub struct Proxy { … }`; `Proxy::bind(cfg: ProxyConfig, deps: ProxyDeps) -> Result<Proxy, ProxyError>` where `pub struct ProxyDeps { pub issuer: Arc<LeafIssuer>, pub ca_der: Vec<u8>, pub ca_pem: String, pub transport: Arc<Transport>, pub sink: Arc<dyn ExchangeSink>, pub handler: Arc<dyn Handler> }`; `Proxy::local_addr(&self) -> SocketAddr`; `Proxy::run(self, shutdown: tokio::sync::watch::Receiver<bool>) -> Result<(), ProxyError>` (accept loop; on shutdown stop accepting, then await live connection tasks up to 5 s); `Proxy::counters(&self) -> Arc<Counters>`.
  - `pub struct Counters { pub accepted: AtomicU64, pub refused_max_connections: AtomicU64, pub refused_per_ip: AtomicU64, pub refused_pending_connect: AtomicU64, pub auth_failures: AtomicU64, pub handshake_rejections: AtomicU64, pub tunnels: AtomicU64, pub exchanges: AtomicU64, pub sink_full: AtomicU64 }`.
  - `pub(crate) enum FirstRequest { Connect { authority: String, port: u16, head: RawMessage }, Absolute { host: String, port: u16, tls: bool, head: RawMessage, leftover: Bytes }, Local { head: RawMessage }, Refused { status: u16, reason: &'static str } }`; `pub(crate) async fn read_first_request(io: &mut dyn Conn, limits: &ProxyLimits) -> Result<(FirstRequest, Bytes /*consumed head bytes*/), ProxyError>` applying the header-read timeout.
  - Authority validation `pub(crate) fn valid_authority(host: &str) -> bool`: ≤ 253 bytes, labels of `[A-Za-z0-9-]` not starting/ending with `-`, or an IPv4 literal, or a bracketed IPv6 literal.
  - Proxy auth: `Proxy-Authorization: Basic base64(user:pass)` compared in constant time; failure → 407 with `Proxy-Authenticate: Basic realm="crosswire"`, counter, and the connection closed; on loopback bind auth is enforced only when configured.
  - Info page (`info_page.rs`): `GET /` → `text/html` built from a `const` template with no scripts (the CA install steps, links to `/ca.der`, `/ca.pem`, `/ca.spki`), `GET /ca.der` → `application/pkix-cert`, `GET /ca.pem` → `application/x-pem-file`, `GET /ca.spki` → `text/plain` (SPKI SHA-256 base64); anything else on the local host → 404. Served when the request is origin-form and `Host` is `crosswire`, `crosswire:PORT`, or the listener's own `host:port`. Response headers: `X-Content-Type-Options: nosniff`, `Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'`, `Referrer-Policy: no-referrer`.
  - Per-connection flow in `listener.rs`: caps checked before spawning (max connections; per-client-IP for non-loopback peers; pending CONNECT = connections between accept and tunnel-established); then `read_first_request`; `Local` → info page then close; `Refused` → generated 400 then close; `Connect` → Task 4's `mitm::serve_connect`; `Absolute` → Task 5's `relay::serve_plain`.

- [ ] **Step 1: Failing tests** in `tests/front.rs`:
  - `connect_gets_200_connection_established` (reply line exactly `HTTP/1.1 200 Connection established\r\n\r\n`).
  - `origin_form_to_unknown_host_is_refused_400` (body contains "Crosswire is a proxy").
  - `info_page_served_for_host_crosswire` (200, `text/html`, contains `/ca.der`, no `<script`).
  - `ca_der_matches_ca` (bytes equal `ca.cert_der()`), `ca_spki_matches`.
  - `non_loopback_bind_without_flag_fails` (`Proxy::bind` → `NonLoopbackBind`; use `0.0.0.0:0`).
  - `proxy_auth_required_then_accepted` (407 without header, 200 with).
  - `max_connections_refuses_the_next` (limits.max_connections = 2; third connect is closed immediately; `refused_max_connections == 1`).
  - `bad_authority_is_refused` (`CONNECT bad host:443` and a 254-byte name → 400).
- [ ] **Step 2: Run** → fail.
- [ ] **Step 3: Implement** listener, front, info page, counters. The harness in `tests/common` builds a CA (`Ca::generate`), `LeafIssuer`, a `Transport` with `RustlsBackend::with_extra_roots(vec![test server CA])` and a `DestinationGate` allowing `127.0.0.1:<port>` of the test upstream (the gate refuses loopback otherwise), and a `TestSink` (`Mutex<Vec<Exchange>>`, `Vec<Marker>`). Tunnel and relay entry points are stubs that close the connection until Tasks 4 and 5.
- [ ] **Step 4: Run** → pass; `just lint`.
- [ ] **Step 5: Commit** `feat(proxy): listener with connection caps, CONNECT/absolute-form front end, info page`.

---

### Task 4: TLS termination, SNI peek, passthrough tunnels, handshake-rejection counter

**Files:**
- Create: `crates/crosswire-proxy/src/mitm.rs`, `src/sni.rs`, `src/tunnel.rs`, `src/rejections.rs`
- Test: `crates/crosswire-proxy/tests/mitm.rs`, unit tests in `sni.rs`, `rejections.rs`

**Interfaces:**
- Consumes: `LeafIssuer::issue(&ServerName<'_>) -> Result<Arc<CertifiedKey>, CaError>`; `FirstRequest::Connect`.
- Produces:
  - `pub fn parse_sni(client_hello_record: &[u8]) -> Result<Option<String>, SniError>` (pure; TLS record header + handshake + ClientHello, reads only the `server_name` extension; `SniError::{Incomplete, NotClientHello, Malformed}`; input capped at 16 KiB by the caller). `pub(crate) async fn peek_sni(tcp: &TcpStream, timeout: Duration) -> Result<Option<String>, ProxyError>` using `TcpStream::peek` into a 16 KiB buffer, retried until a full record or the timeout.
  - `pub(crate) fn passthrough_matches(list: &[String], sni: &str) -> bool` (exact, case-insensitive; `.example.com` matches any subdomain, not the apex).
  - `pub(crate) async fn serve_connect(conn: ConnCtx, authority: String, port: u16) -> Result<(), ProxyError>`: write `200 Connection established`; `peek_sni` (5 s); if passthrough → `tunnel::run(client, upstream_tcp, TunnelReason::SniPassthrough)` where upstream is `Transport::connect(&Target::plain(authority, port))` (gate applies; failure closes the client); else `accept_tls`: a per-connection `rustls::ServerConfig` from `ServerConfig::builder_with_provider(ring provider).with_protocol_versions(&[&TLS13, &TLS12]).with_no_client_auth().with_cert_resolver(Arc<FixedCert>)` where the cert comes from `issuer.issue(&ServerName::try_from(sni.unwrap_or(authority)))` (IP literals produce an IP SAN via `ServerName::IpAddress`), ALPN `["http/1.1"]`, handshake under `limits.tls_handshake`; on handshake error record the rejection (below) and close; on success hand the TLS stream to Task 5's `relay::run` with `Target::tls(authority, port, sni_or_authority, verify_mode(authority))` where `verify_mode` is `AcceptInvalid` only when `authority` ∈ `accept_invalid_cert`.
  - `tunnel.rs`: `pub(crate) async fn run(a: Box<dyn Conn>, b: Box<dyn Conn>, reason: TunnelReason, idle: Option<Duration>, ctx: RecordCtx) -> TunnelStats` copying both directions with `tokio::io::copy_bidirectional`-equivalent loops (64 KiB buffers), counting bytes, closing on `tunnel_idle`; on close submits one `Exchange` whose request is `Message::new(MessageKind::Tunnel, Fidelity::Opaque, Head::Tunnel { bytes_in, bytes_out, duration_ms, reason }, FramingRecord::None, Body::None, conn_info)` and no response.
  - `rejections.rs`: `pub struct RejectionTracker` (per-host sliding window: `VecDeque<Instant>` per host, capped at 1,024 hosts LRU); `fn record(&self, host: &str, now: Instant) -> bool` returns true when the count in `rejection_window` reaches `rejections_per_window`; the caller then `submit_marker(Marker::new(MarkerKind::Error, "Browser rejected the Crosswire certificate for {host} — is the CA trusted?"))` once per window and increments `handshake_rejections`. No auto-passthrough (Global Constraints).

- [ ] **Step 1: Failing tests**: `sni.rs` units `parses_sni_from_real_client_hello` (bytes captured from rustls: build a ClientHello with `rustls::ClientConnection` writing into a buffer), `no_sni_returns_none`, `truncated_is_incomplete`, `not_a_client_hello_is_rejected` (first byte 0x17). `tests/mitm.rs`: `mitm_leaf_has_host_san_and_is_trusted_by_ca` (client trusts the test CA; connects via CONNECT to `example.test`; handshake succeeds; leaf SAN contains `example.test`), `ip_literal_gets_ip_san` (`CONNECT 127.0.0.1:PORT`), `passthrough_host_is_tunnelled_and_recorded` (SNI `pass.test` in list; the client's TLS session is with the upstream's own cert; the sink receives one `Head::Tunnel { reason: SniPassthrough }` exchange with `bytes_in > 0 && bytes_out > 0`), `three_rejections_in_window_emit_error_marker` (client without the CA aborts 3 handshakes; `handshake_rejections == 3`; one `MarkerKind::Error` marker whose detail contains the host), `passthrough_suffix_rule` (unit). `rejections.rs` unit `window_slides`.
- [ ] **Step 2: Run** → fail.
- [ ] **Step 3: Implement.** The DNS for `example.test` in tests: the harness's `Transport` is built with a `DestinationGate` allowlisting `127.0.0.1:<upstream port>`; to make `example.test` resolve, the harness uses `Target::tls("127.0.0.1", port, "example.test", …)` — expose `ProxyDeps.resolve_override: Option<Arc<dyn Fn(&str) -> Option<IpAddr> + Send + Sync>>` used by the proxy when building `Target` (host rewritten to the override IP; SNI stays the authority). Document it as test-only wiring (`#[doc(hidden)]`).
- [ ] **Step 4: Run** → pass; `just lint`.
- [ ] **Step 5: Commit** `feat(proxy): TLS termination with issued leaves, SNI peek and passthrough tunnels`.

---

### Task 5: Relay core — one exchange, keep-alive, pipelining, rewrites and diffs

**Files:**
- Create: `crates/crosswire-proxy/src/relay.rs`, `src/rewrite.rs`, `src/pump.rs`
- Test: `crates/crosswire-proxy/tests/relay_plain.rs`, `tests/relay_tls.rs`, `tests/common/upstream.rs` (raw TCP/TLS upstream that records received bytes and replies with scripted bytes, optionally in fragments)

**Interfaces:**
- Consumes: `Parser::request/response/feed/feed_eof/reset/reset_response/body_diagnostics`, `Progress::{NeedMore, Head, BodyData{raw,data}, ChunkExt, Trailers, Done}`, `Step{consumed,event}`, `Transport::connect`, `Connection::{io_mut, info, into_parts}`, `Handler`, Task 6's `Recorder` (introduced here as a trait object `RecordCtx` with a stub implementation until Task 6).
- Produces:
  - `pub(crate) async fn run(client: Box<dyn Conn>, first: Option<(RawMessage, Bytes)>, target: Target, ctx: ConnCtx) -> Result<(), ProxyError>`: connects upstream through `Transport` on the first request (connect errors handled in Task 7), then loops `exchange()` until either side ends the connection.
  - `pump.rs`: `pub(crate) struct Pump { buf: BytesMut /*≤ 64 KiB read buffer*/, pending: usize }` with `async fn step(&mut self, from: &mut dyn Conn, parser: &mut Parser, to: &mut dyn Conn, capture: &mut Capture, on_head: impl FnMut(&RawMessage) -> Decision) -> Result<PumpEnd, ProxyError>` implementing the README re-present loop: feed `pending`, advance by `consumed`, **write exactly the consumed bytes to `to` before reading more** (bounded buffering; the write completes before the next read), copy consumed bytes into `capture`, read only on `NeedMore`, `feed_eof` on EOF. `PumpEnd::{Done { leftover: Bytes }, Eof, Dropped }`.
  - `rewrite.rs`: `pub(crate) fn rewrite_request_head(head: &RawMessage, mode: Mode) -> (Bytes, Vec<Diff>)` where `Mode::{Tunnel, Plain { host: String }}`: in both modes delete `Proxy-Connection` and `Proxy-Authorization` header lines (`ChangeOp::Delete`, `ChangeReason::HopByHop`); in `Plain`, replace the absolute-form target with origin-form (`ChangeOp::Replace`, span = the target span, `ChangeReason::Custom("absolute-form")`) and, if `Host` is absent, insert `Host: {host}\r\n` after the start line (`Insert`, `Custom("host-from-target")`). Spans are relative to the original head bytes. Everything else is byte-identical.
  - Exchange sequence inside `exchange()`: request head (via `Pump` on the client with the request parser; `Handler::on_request_head` → `Drop` closes both sides) → rewrite → write to upstream → request body pump client→upstream (`on_request_body_chunk`) → `resp_parser.reset_response(ResponseContext::for_method(method))` → response pump upstream→client, forwarding 1xx heads (≠101) as they arrive and keeping their bytes for `interim`; ttfb = first upstream byte after the last request byte; → `Done` → hand `ExchangeDraft` to the recorder (never awaited) → reuse decision: close when request or response carries `Connection: close` (token list, case-insensitive), the response is HTTP/1.0 without `keep-alive`, framing was `UntilClose`, or `PumpEnd::Eof`; else next exchange with any `leftover` re-presented first (pipelining, serial; more than `max_pipelined` queued request heads → close with `Limit("pipelining")`).
  - `pub(crate) struct ExchangeDraft { pub req_head: RawMessage, pub req_rewritten: Bytes, pub diffs: Vec<Diff>, pub req_body: CaptureResult, pub resp_head: Option<RawMessage>, pub interim: Vec<Bytes>, pub resp_body: CaptureResult, pub timing: Timing, pub info: ConnectionInfo, pub state: ExchangeState, pub generated: Option<Message>, pub body_diagnostics: (Vec<Diagnostic>, Vec<Diagnostic>) }` (`CaptureResult` is Task 6's; here a placeholder struct holding `Bytes` + len + hash).

- [ ] **Step 1: Failing tests** (`tests/relay_plain.rs`, absolute-form over plain HTTP; `tests/relay_tls.rs`, CONNECT + MITM; the TLS file runs the same scenarios through a helper):
  - `get_round_trip_is_byte_exact`: upstream received `GET /p?q=1 HTTP/1.1\r\nHost: u.test\r\nUser-Agent: x\r\n\r\n` (origin-form, `Proxy-Connection` removed); client received the scripted response bytes exactly; recorded request diffs: one `Replace` (`absolute-form`) and one `Delete` (`HopByHop`) with spans pointing at the original bytes.
  - `keep_alive_two_requests_same_upstream_connection`: upstream saw both on one socket; second recorded exchange has `reused == true` and the same `conn_id`.
  - `connection_close_from_server_ends_relay`.
  - `pipelined_requests_are_served_serially_in_order` (two heads in one write; responses in order; two exchanges).
  - `request_body_content_length_forwarded_exactly` (POST 70 KiB in 3 writes).
  - `chunked_response_with_extensions_and_trailers_is_byte_exact` (chunk `5;ext=1\r\nhello\r\n`, trailers, ends `0\r\n\r\n`; client bytes identical; recorded response diagnostics contain the chunk-ext code and the stored body equals the decoded data).
  - `head_and_304_responses_are_framed_without_body` then a second request on the same connection succeeds.
  - `http10_until_close_response_ends_connection`.
  - `expect_100_continue_interim_is_forwarded_and_recorded` (upstream sends `100 Continue` then `200`; recorded `interim.len() == 1`).
  - `host_header_differs_from_authority_is_forwarded_unchanged` (Review Focus 1; over TLS: `CONNECT a.test`, request `Host: b.test`; upstream (the `a.test` server) receives `Host: b.test`; stored `req_host == "b.test"` via `Store` in `tests/record.rs` later — here assert the recorded request bytes contain it).
  - `handler_drop_closes_without_forwarding` (custom handler returns `Drop` on request head; upstream received nothing).
  - `pipelining_depth_over_limit_closes` (`max_pipelined = 2`, 3 heads in one write → connection closed, `Limit` counted).
- [ ] **Step 2: Run** → fail.
- [ ] **Step 3: Implement** `pump.rs`, `rewrite.rs`, `relay.rs`; wire `serve_plain` (Task 3) and the post-handshake path (Task 4) to `relay::run`. The upstream `Target` for plain absolute-form is `Target::plain(host, port)`; for `https://` absolute-form (rare, allowed) `Target::tls(host, port, host, verify_mode(host))`.
- [ ] **Step 4: Run** → pass; `just lint`.
- [ ] **Step 5: Commit** `feat(proxy): HTTP/1.1 relay with keep-alive, pipelining, origin-form rewrite and diffs`.

---

### Task 6: Recording — capture buffer, caps, in-flight budget, exchange build, submit

**Files:**
- Create: `crates/crosswire-proxy/src/capture.rs`, `src/record.rs`
- Modify: `src/relay.rs` (replace the placeholder `CaptureResult`), `src/lib.rs`
- Test: `crates/crosswire-proxy/tests/record.rs` (uses a real `Store` in a tempdir), unit tests in `capture.rs`

**Interfaces:**
- Consumes: `Body::inline`, `Capture`, `Message::http1_raw(...).with_diffs(...).with_diagnostics(...).with_interim(...)`, `Exchange::new/set_response/set_timing/set_state`, `ExchangeSink::submit`, Task 1's `relay` fields.
- Produces:
  - `pub struct Budget { limit: u64, used: AtomicU64 }`, `Budget::try_reserve(&self, n: u64) -> Option<Reservation>` (RAII release), shared across the listener (`body_budget` 256 MiB).
  - `pub(crate) struct CaptureBuf { kept: BytesMut, cap: u64, total: u64, hasher: Sha256, dropped: bool, reservation: Option<Reservation> }` with `push(&mut self, bytes: &[u8])` (always hashes and counts; appends only while `kept.len() < cap` and not dropped; reserves budget in 64 KiB steps, sets `dropped` and clears `kept` when the budget refuses) and `finish(self) -> CaptureResult { bytes: Bytes, truncated_at: Option<u64>, capture_dropped: bool, relay: Capture { len, sha256 } }`.
  - `pub(crate) struct Recorder { sink: Arc<dyn ExchangeSink>, counters: Arc<Counters>, parse: ParserConfig }`; `fn record(&self, draft: ExchangeDraft)`: builds the request `Message::http1_raw(&parse(req_rewritten), body, info)` — the head is re-parsed from the **rewritten** bytes with the proxy's `ParserConfig` so spans match the canonical request — `.with_diffs(draft.diffs).with_diagnostics(head + body diagnostics)`; the response `Message::http1_raw(&resp_head, body, info).with_diagnostics(...).with_interim(draft.interim)` or `draft.generated`; `Exchange::new(request)` + response + timing + `state`; then `tokio::task::spawn_blocking(move || sink.submit(exchange))` and, on `Err(SinkError::Full)`, increments `sink_full` and logs at `warn` once per 10 s (no bytes).
  - Body representation: every captured body is `Body::inline(bytes)` plus the new fields; bodies above `inline_memory` are still submitted inline (the store packs them); `capture_cap` bounds the kept bytes. ADR-0003 records that the separate pack-writer stream of §7.1 is not built in alpha.1.

- [ ] **Step 1: Failing tests**: `capture.rs` units `keeps_prefix_counts_all_hashes_all` (cap 1 KiB, push 3 KiB → kept 1024, `truncated_at == Some(1024)`, `relay.len == 3072`, hash of all 3 KiB), `budget_refusal_marks_dropped` (budget 1 KiB, push 2 KiB → `capture_dropped`, `kept.is_empty()`, len/hash still right), `reservation_released_on_finish`. `tests/record.rs` (real `Store`): `exchange_is_queryable_with_both_bodies` (`Store::get` returns request + response; `get_body(Req/Resp)` equal the wire bodies; `req_host` filter finds it), `truncated_response_is_recorded_with_relay_capture` (`capture_cap = 1024`, 70 KiB response; stored body is 1024 bytes; JSON shows `truncated_at: 1024`, `relay.len == 71680`; the client still received all 70 KiB), `body_diagnostics_are_recorded` (chunk extension code present on the response), `interim_heads_round_trip_through_store`, `slow_client_does_not_buffer_upstream` (Review Focus 3: client reads 1 KiB every 20 ms for 2 s while upstream offers 50 MiB; assert the upstream's unacked written bytes stayed under 1 MiB — the upstream helper reports how many bytes the proxy had read; and the proxy's `Pump` buffer is 64 KiB by construction), `sink_full_never_stalls_relay` (a `TestSink` returning `Full`; 3 exchanges complete; `sink_full == 3`).
- [ ] **Step 2: Run** → fail.
- [ ] **Step 3: Implement**; make `ExchangeDraft` carry `CaptureResult`.
- [ ] **Step 4: Run** → pass; `just lint`.
- [ ] **Step 5: Commit** `feat(proxy): bounded capture with cap, hash, in-flight budget; record to the sink`.

---

### Task 7: Failure paths — connect/TLS/gate errors, timeouts, parse errors, undelimitable and 101 tunnels

**Files:**
- Modify: `crates/crosswire-proxy/src/relay.rs`, `src/tunnel.rs`, `src/listener.rs`
- Test: `crates/crosswire-proxy/tests/failures.rs`

**Interfaces:**
- Consumes: `generated(...)`, `ExchangeSink::submit_marker`, `ClientError::{Dns, Refused, Tls, Timeout, Io}`, `ParseError`, `Head::Tunnel`, `TunnelReason::{Ws, Undelimitable}`.
- Produces (behaviour, all recorded as `ExchangeState::Error` exchanges with the request as captured so far and the generated response when one was sent):
  - Upstream connect failure (`Dns`, `Io`) → 502 `BAD_GATEWAY_CONNECT`; `Refused(GateRefusal)` → 502 `REFUSED_DESTINATION` (refusal text ≤ 1 KiB); `Tls` with verify → 502 `BAD_GATEWAY_TLS`. In the CONNECT path the upstream connection is made after the client's first request head, so the generated response goes inside the tunnel.
  - Timeouts: header read (client) 30 s → close silently (no request yet); upstream head wait `idle` 60 s → 504 `GATEWAY_TIMEOUT` phase `"response head"`; body idle 60 s or `total` 300 s during the response → close both (headers already sent; marker `MarkerKind::Error` detail `"timeout: {phase} {host}:{port}"`).
  - Client `ParseError` (any) → generated 400 `"Crosswire could not parse the request: {code}"` and `submit_marker(Marker::new(MarkerKind::LimitExceeded | Error, …))` (`LimitExceeded` for `HeadTooLarge`, `TooManyHeaders`, `LineTooLong`, `TooManyChunks`, `TrailerTooLarge`; `Error` otherwise), then close.
  - Upstream `ParseError` on the response → the bytes consumed so far have already been forwarded; the rest of the connection becomes a raw tunnel (`TunnelReason::Undelimitable`) until either side closes; the current exchange is recorded with `state: Error` and a diagnostic-free note in the marker `"undelimitable response from {host}:{port}: {code}"`.
  - 101 Switching Protocols: the response head is forwarded and recorded as the exchange; the remainder is a tunnel exchange with `TunnelReason::Ws` and counters; `tunnel_idle` applies when set.
  - Client disconnect mid-request-body (Review Focus 4) and upstream close mid-body (Review Focus 2): record the partial exchange as `Error`, close the other side, release counters and budget.

- [ ] **Step 1: Failing tests** in `tests/failures.rs`: `upstream_connect_refused_gives_502_and_error_exchange`, `connect_to_own_listener_is_refused_with_hint` (Review Focus 5: `CONNECT 127.0.0.1:<proxy port>`; body contains `--allow 127.0.0.1:`; sink has one `Error` exchange; the upstream counter shows zero connects), `upstream_bad_chain_is_502_without_override_and_relayed_with_it` (two proxies: one default, one with `accept_invalid_cert = [host]`; the second records `ChainResult::Overridden { reason }` non-empty and `cert_override == true`), `response_head_timeout_gives_504` (`idle = 300 ms`), `request_head_over_limit_gives_400_and_limit_marker` (70 KiB head), `undelimitable_response_falls_to_tunnel` (upstream sends a garbage status line then bytes; client receives all bytes; sink has an `Error` exchange then a `Tunnel { reason: Undelimitable }` exchange), `switching_protocols_records_upgrade_then_ws_tunnel`, `upstream_close_mid_body_records_error_and_closes_client`, `client_disconnect_mid_request_body_records_partial_request`, `tunnel_idle_closes` (`tunnel_idle = 200 ms`).
- [ ] **Step 2: Run** → fail.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** → pass; `just lint`; run `cargo test -p crosswire-proxy` three times to check for timing flakes (timeouts in tests use ≥ 200 ms values and assert with 5× margins).
- [ ] **Step 5: Commit** `feat(proxy): generated errors, timeouts, parse-error handling, undelimitable and upgrade tunnels`.

---

### Task 8: `crosswire serve` and `crosswire ca` (the binary)

**Files:**
- Create: `crates/crosswire-server/src/main.rs`, `src/cli.rs`, `src/serve.rs`, `src/ca_cmd.rs`, `src/paths.rs`
- Test: `crates/crosswire-server/tests/cli.rs`

**Interfaces:**
- Consumes: `Proxy::bind/run/local_addr/counters`, `ProxyConfig`, `ProxyDeps`, `Ca::load_or_generate(&FileKeyStore, suffix)`, `LeafIssuer::new(Arc<Ca>, IssuerLimits::default())`, `RustlsBackend::webpki_roots()`, `DestinationGate::new(own_ports)`/`allow(host, port)`, `Transport::new`, `Project::open_or_create`, `Store::open/sink/audit/close`, `StoreConfig::default()`.
- Produces:
  - CLI (`clap` derive): `crosswire serve --project PATH [--proxy-bind ADDR (default 127.0.0.1:8080)] [--allow-non-loopback] [--proxy-auth USER:PASS] [--passthrough HOST]... [--allow HOST:PORT]... [--accept-invalid-cert HOST]... [--ca-dir PATH] [--capture-cap BYTES] [--log LEVEL]`; `crosswire ca export --format der|pem [--out PATH] [--ca-dir PATH]`; `crosswire ca spki [--ca-dir PATH]`; `crosswire ca path`.
  - `paths.rs`: `fn default_ca_dir() -> PathBuf` = `dirs::config_dir()/crosswire/ca` (error with a clear message when no config dir exists).
  - `serve.rs`: order: validate config → open project (`Project::open_or_create`) → `Store::open` → CA `load_or_generate` (audit `ca.generate` when created, actor `cli`) → gate with `own_ports = [proxy port]` plus `--allow` entries (audit `allow.add` each) → `--accept-invalid-cert` audited `cert.override.config` → `Proxy::bind` → audit `listener.bind` (target = bound addr) → print exactly one line `proxy listening on {addr}` and one line `ca page at http://crosswire/ (via the proxy) or http://{addr}/` → `Proxy::run` with a `watch` channel flipped by `tokio::signal::ctrl_c` → after run returns, `Store::close` (clean shutdown) → audit `listener.stop` is not possible after close, so write it before closing. Logging: `tracing_subscriber::fmt().with_env_filter(EnvFilter::try_from_default_env().or(--log))`, default `info`; the proxy crate logs at `debug` for per-connection events and never logs bytes.
  - Exit codes: 2 for config errors (non-loopback without flag, bad `--allow`), 1 for runtime failures, 0 on Ctrl-C.

- [ ] **Step 1: Failing tests** in `tests/cli.rs` (spawn `env!("CARGO_BIN_EXE_crosswire")`): `ca_export_der_matches_pem_and_spki` (export both to a tempdir `--ca-dir`; DER parsed from the PEM equals the DER file; `ca spki` equals the SPKI computed from the DER with `x509-parser` + sha2 in the test), `serve_records_a_tls_fetch_and_closes_cleanly` (`--proxy-bind 127.0.0.1:0 --project tmp --ca-dir tmp --allow 127.0.0.1:<upstream>`; parse the `proxy listening on` line; an in-process TLS upstream with a self-signed cert → pass `--accept-invalid-cert 127.0.0.1`; CONNECT + TLS with the exported CA trusted; fetch; send SIGINT (`kill -INT`); wait for exit 0; `Project::open` succeeds without an unclean-restart gap marker and `Store::list` has the exchange with `req_host == "127.0.0.1:<port>"`; `audit.jsonl` contains `listener.bind` and `cert.override.config`), `serve_refuses_non_loopback_without_flag` (exit 2, stderr mentions `--allow-non-loopback`).
- [ ] **Step 2: Run** → fail.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** → pass; `just lint`; `cargo run -p crosswire-server -- --help` shows the three subcommands.
- [ ] **Step 5: Commit** `feat(server): crosswire serve and crosswire ca subcommands`.

---

### Task 9: Corpus relay golden and hostile `Limits` fixtures

**Files:**
- Create: `crates/crosswire-proxy/tests/corpus_relay.rs`, `tests/limits.rs`, `tests/common/corpus.rs` (loads `corpus/h1/cases/*.msg` + `*.expect.json`, reusing the h1 harness's reader if it is public; otherwise a 40-line loader)
- Modify: `corpus/h1/README.md` (one paragraph: the proxy golden reuses the cases)

**Interfaces:**
- Consumes: the whole proxy; `.expect.json` fields `kind` (request/response), `codes` (diagnostic codes) and `error` (expected `ParseError`, if any).

- [ ] **Step 1: Write `corpus_relay.rs`**: one test `every_corpus_case_relays_byte_exact_and_records_its_diagnostics` iterating all cases × two write patterns (whole; 7-byte fragments with 1 ms gaps), through the CONNECT/MITM path (origin-form bytes go verbatim): for request cases the client writes the case bytes and the upstream must receive them unchanged (except `Proxy-*` lines, which the corpus does not contain) and reply with a fixed `200`; for response cases the upstream replies with the case bytes after a fixed `GET` and the client must receive them unchanged. Assert the recorded message's diagnostic code set equals `expect.codes`, the recorded head bytes equal the wire head, and the exchange count at the end ≥ 200 (print the number). Cases whose `expect.error` is set must produce the Task 7 behaviour (400 for request errors; tunnel fallback for response errors) and are counted separately; the assertion lists which cases took which path. Head-only corpus cases with a declared body use `expect.json`'s body bytes if present, else no body.
- [ ] **Step 2: Write `limits.rs`**, one test per row, each with the limit configured small where possible and asserting the row's observable: `head_too_large_400_limit_marker`, `too_many_headers`, `line_too_long`, `chunk_size_line_too_long`, `too_many_chunks` (`max_chunks = 8`), `pending_connect_cap` (`max_pending_connect = 2`, three clients that send `CONNECT` but no ClientHello → the third is refused, counter), `per_ip_cap_applies_to_non_loopback_only` (loopback clients are never capped: open 70 loopback connections with `max_per_client_ip = 64`), `sni_too_long_rejects_connect` (254-byte authority → 400 before any TLS), `tunnel_idle_closes_passthrough`, `total_timeout_closes_trickle` (`total = 500 ms`, upstream sends one byte every 100 ms forever), `rss_flat_under_large_until_close_response` (Linux only via `/proc/self/status` `VmRSS`; 256 MiB until-close response with `capture_cap = 1 MiB`; RSS growth < 64 MiB; on other OSes the test asserts only the capture bookkeeping).
- [ ] **Step 3: Run** `cargo test -p crosswire-proxy --test corpus_relay --test limits` → pass (fix relay bugs the corpus finds in this task; each fix is its own `fix(proxy):` commit with the case name in the body).
- [ ] **Step 4: Commit** `test(proxy): corpus relay golden (≥200 captures) and hostile Limits fixtures`.

---

### Task 10: Real-browser session test and CI wiring

**Files:**
- Create: `crates/crosswire-proxy/tests/browser.rs`
- Modify: `.github/workflows/ci.yml`, `justfile`

- [ ] **Step 1: Write `tests/browser.rs`**: `#[tokio::test] #[ignore]` `chromium_session_is_recorded`: skip with a message unless `CROSSWIRE_BROWSER` (path to a Chromium binary) is set; start the in-process proxy harness with an in-process TLS upstream serving `/` (`<html>hi</html>`) under a test CA added to the proxy's `Transport` roots; launch the browser with `--headless=new --no-sandbox --disable-gpu --proxy-server=127.0.0.1:{port} --proxy-bypass-list=<-loopback> --ignore-certificate-errors-spki-list={CA SPKI} --user-data-dir={tmp} --dump-dom https://browser.test/` (the harness's `resolve_override` maps `browser.test` to the upstream), wait ≤ 30 s for exit; assert stdout contains `hi` and the store has an exchange with `req_host == "browser.test"`, `tls_version` set, `fidelity == Raw`. (The headless browser also issues background requests; the test tolerates extra exchanges.)
- [ ] **Step 2: justfile**: `browser-test:` → `CROSSWIRE_BROWSER=${CROSSWIRE_BROWSER:-$(command -v google-chrome || command -v chromium || command -v chromium-browser)} cargo test --locked -p crosswire-proxy --test browser -- --ignored`.
- [ ] **Step 3: CI**: in the `check` job add a step after `just test`, ubuntu only (`if: matrix.os == 'ubuntu-latest'`): `run: just browser-test` (GitHub's ubuntu image has Google Chrome). Keep the step non-fatal for macOS by the `if`.
- [ ] **Step 4: Run** locally with `CROSSWIRE_BROWSER=/Applications/Google\ Chrome.app/Contents/MacOS/Google\ Chrome just browser-test` → pass (if no Chrome locally, say so in the report; CI proves it).
- [ ] **Step 5: Commit** `test(proxy): headless Chromium session recorded end to end; CI browser job`.

---

### Task 11: Docs, ADR-0003, format and log

**Files:**
- Create: `crates/crosswire-proxy/README.md`, `crates/crosswire-server/README.md`, `docs/adr/0003-proxy-relay.md`, `docs/ca-install.md`
- Modify: `docs/format.md` (proxy-written markers, `interim`, body capture fields if Task 1 left anything out), `docs/log.md` (rows "proxy 0".."proxy 11", `n/m`; velocity summary line counts via `tokei 15.0.0 crates corpus --exclude '*.msg' --exclude target -t Rust`), root `README.md` (status: alpha.1 headless recorder usable; quick start: `cargo run -p crosswire-server -- serve --project ./demo`, open `http://crosswire/` through the proxy for the CA; what is not there: no UI, no repeater, no intercept), spec `§17` (strike what alpha.1 built: proxy listener, Handler (Forward/Drop), SNI passthrough, capture_cap/in-flight budget, `crosswire serve`; add what stays deferred: auto-passthrough (needs scope), keychain + trust installation, pack-writer streaming, retention/purge, proxy auth beyond Basic).

- [ ] **Step 1: Write the docs**; every number cross-checked against `limits.rs` and `config.rs`. ADR-0003 "Proxy relay is a byte pump with a bounded capture window" — Context (fidelity + never-block-on-store), Decision (two parsers, forward exactly consumed bytes, write-before-read backpressure, capture buffer with cap/hash/budget, `spawn_blocking` submit, generated responses recorded as `Fidelity::Generated`, no `Session` on the proxy path), Alternatives (hyper-based proxy: rejected, normalises; using `Session::send`: rejected, buffers whole requests; separate pack-writer stream per §7.1: deferred, inline-through-store is enough for alpha.1 given the 64 MiB queue), Consequences (a 50 MiB body occupies the queue; the journal absorbs bursts; intercept needs `Hold` in `Decision`, additive). `docs/ca-install.md`: Chromium flag (`--ignore-certificate-errors-spki-list`), Chrome/Edge system trust per OS, Firefox import, Android 7+ user-CA limitation, iOS profile; the text served by the info page links here.
- [ ] **Step 2: Verify** `just doc`, `just test`, `cargo deny check`, `just lint`.
- [ ] **Step 3: Commit** `docs(proxy): READMEs, ADR-0003 relay design, CA install guide, format and log`.

---

## Self-review notes

- **Spec coverage:** §7.1 steps 1–6 → Tasks 3, 4, 5, 6 (step 3's auto-passthrough deliberately off, Global Constraints); §7.1 body model → Task 6 (inline-through-store instead of a pack-writer stream, ADR-0003); WebSocket-as-opaque → Task 7; Handler → Task 2 (Forward/Drop only); §6.0 connection model, no-body responses, invalid CL, framing resolution, undelimitable → Tasks 5, 7, 9; §11.2 listener shape, origin-form refusal, info page, non-loopback auth → Task 3; destination gate → reused from client, Task 7 test; §11.3 leaves with IP SAN, SNI ≤ 253, mint limits → Task 4 (keychain/trust install stay deferred); §11.6 rows that apply to the proxy → Tasks 2, 7, 9; §11.9 → Tasks 4, 7; §12 `crosswire serve`, CA page → Tasks 3, 8; §13 corpus golden, hostile fixtures, RSS → Task 9; browser session (alpha.1 mini-DoD) → Task 10; §9.3 never-block → Task 6. **Deferred (named in §17 by Task 11):** auto-passthrough, keychain, OS trust install, `crosswire browser` (alpha.2), retention/purge, pack-writer streaming, Hold/Modify.
- **Plan 2 "must change" list:** capture window not whole-response (Task 6); error exchanges on failures (Task 7); target_addr/peer_addr (Task 1); body diagnostics (Task 6); interim heads (Tasks 1, 5); session parser config → the proxy's `ParserConfig` everywhere (Task 6); redaction stays on the way out per §11.4 (not at rest; noted in ADR-0003).
- **Type consistency:** `ProxyLimits`, `ProxyConfig`, `ProxyDeps`, `Counters`, `Decision`, `Handler` (Task 2/3) are used by name in Tasks 4–8; `ExchangeDraft` and `CaptureResult` (Tasks 5/6); `generated(...)` (Task 2) in Tasks 3, 7; `submit_marker`, `Capture`, `Body::inline`, `with_interim`, `with_peer_addr`, `Overridden { reason }` (Task 1) in Tasks 4–8.
- **Review Focus:** each of the five has a named test in Tasks 5, 6, 7, 9.
- **Proportion:** signatures, test names and assertions only; no bodies except the fixed copy strings and the exact generated-response shape, which the spec pins.
