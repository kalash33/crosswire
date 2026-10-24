# Crosswire Sidecar Backend Implementation Plan (Plan 4)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Everything 0.1.0-alpha.2 needs behind the UI: the h1 edit engine, the scope crate, request intercept, the repeater, a credentialed REST + WebSocket API with OpenAPI, the `crosswire browser` launcher, and the §13 security gates that can be proven without a UI. Plan 5 builds the web UI on this API.

**Architecture:** New library crates `crosswire-scope` (grammar + evaluator + importer), `crosswire-tools` (repeater send and recording), `crosswire-api` (axum router, auth, events), plus additive changes: `apply_edits` in `crosswire-h1`, a `control` write path and three tables (settings, tokens, drafts) in `crosswire-store`, and an async `Handler` with `Hold`/`Modify` in `crosswire-proxy` driven by an `Intercept` service. `crosswire-server` composes them: `serve` binds the API on loopback, prints the one-time bootstrap URL, and `crosswire browser`/`token` subcommands arrive. The spec's principle 4 holds: the UI, CLI and later MCP are clients of this one API.

**Tech Stack:** Rust 1.88 (MSRV), tokio, axum 0.8 (`ws`), tower-http (`trace` only), utoipa 5 + utoipa-axum (OpenAPI), regex 1, ipnet 2, rand 0.9, subtle 2, sha2; existing workspace crates.

**Spec:** `docs/superpowers/specs/2026-10-22-crosswire-design.md` — §4 (principles 4, 6, 8), §4.6–4.7 (scope meanings and model), §5.1 (`apply_edits`, `Edit`, `Edited`, `EditError`, `Change`), §6.0 (fidelity), §7.2 (repeater), §7.3 (intercept), §7.6 (scope import, `crosswire browser`), §8 (API and events), §9.4 (audited actions), §11.2 (listeners and credentials), §11.4 (redaction on the way out), §11.6 Limits (intercept rows, auth-failure rate), §12 (browser launcher, install-to-first-capture), §13 (security gates), §14 (alpha.2 mini-DoD). The pre-0.1.0 backlog carried from Plans 2–3 is folded in where a task touches the area (noted per task).

## Global Constraints

- `rust-version = "1.88"`; `unsafe_code = "forbid"`; `missing_docs = "deny"`; published crates depend only on published-or-workspace crates; no new default features.
- **API listener is loopback only** (`--api-bind` elsewhere prints a warning and still binds); API and proxy are separate sockets (§11.2).
- **Credentials (§11.2, verbatim values):** bootstrap token ≥ 128 bits, single-use, 60 s expiry, rate-limited, accepted only by `POST /api/v1/session`; delivered once in a URL fragment or a 0600 file, never on argv. Session cookie `cw_sid_<port>`, HttpOnly, SameSite=Strict, `Path=/api`, idle expiry 30 min. API tokens scoped `read` | `send` | `admin`, project-bound, hashed server-side (SHA-256 of a 256-bit random token), shown once, individually revocable; constant-time comparison; auth failures rate-limited at 10 per minute per peer and audited.
- **Cookie isolation by port:** every cookie-authenticated request, including GET and the WebSocket upgrade, must carry `Sec-Fetch-Site: same-origin` or an `Origin` exactly equal to the API origin, else 403. The API never emits CORS headers. The WebSocket may alternatively authenticate with a bearer token in its first frame.
- **Request checks:** serve the API only when `Host` is `localhost[:port]` or `127.0.0.1[:port]`; anything else → 421. The only unauthenticated routes: `GET /api/v1/ca.der`, `GET /api/v1/ca.pem`, `POST /api/v1/session`, `GET /api/v1/openapi.json`, `GET /api/v1/health`. Static assets join this list in Plan 5.
- **Scope (§4.6):** everything is recorded by default; out-of-scope traffic is never intercepted; active sends allowed by default with a persistent out-of-scope marker and one audit row per out-of-scope send; `enforce_scope_strictly` (default off) refuses them; empty scope = everything in scope, no prompts, intercept's out-of-scope skip is a no-op. Scope inputs: connection identity (configured host or IP literal, plus SNI) and message identity (`Host`); DNS results never. Evaluate both; record `scope_state: host-mismatch` when they disagree.
- **Scope model (§4.7):** ordered include rules then exclude rules; a rule = host pattern (exact, `*.suffix` matching subdomains only unless `also_apex`, regex, IP literal, CIDR) + optional port set + optional scheme + optional path prefix or regex; first matching exclude wins; no include match → out; no rules → empty. Regex compiled-size limit 10 MiB.
- **Intercept (§7.3, §11.6):** off at project open; default rules skip requests by URL extension (`png jpg jpeg gif webp svg ico woff woff2 ttf css js map`), skip out-of-scope hosts (no-op under an empty scope); `intercept_hold` 4 MiB (larger bodies stream through marked not editable); `intercept_hold_timeout` 120 s (actions: forward | 504); queue depth 64 (overflow actions: forward newest | forward oldest); held bodies are charged to the in-flight budget. Response intercept is on the v0.1 cut list and is out of this plan.
- **Repeater (§7.2):** assisted mode = Content-Length update + CRLF normalisation, each a recorded change; the Content-Length chip never touches a message carrying `Transfer-Encoding` (recorded as skipped); raw mode sends bytes untouched with a best-effort `ResponseContext` falling back to until-close; redirect follow off | in-scope | always with cap 10; results in the `repeater` namespace owned by the tab, linked to the source exchange; `cert_override` surfaced; TTFB and total time recorded.
- **Edit engine (§5.1):** `apply_edits` is the ONE edit entry point; every edit yields new bytes that are re-parsed with the same `ParserConfig`; `Change.span` is in after-coordinates; errors `Overlap`, `OutOfRange`, `BodyNotInline`, `Reparse`.
- **Audited actions in this plan (§9.4):** scope changes, each repeater send (target + mode), out-of-scope sends, session bootstrap, auth failures, token create/revoke/rotate, intercept enable/disable, allowlist changes (exists), browser profile reset.
- **No phoning home (§4 principle 8):** no outbound connection while idle; the launcher opens only the proxy's own CA page.
- **Project format:** schema `user_version` 1 → 2 by migration `0002_sidecar.sql`, additive tables only, documented in `docs/format.md`; a snapshot of `project.db` is taken before migrating.
- Logs never contain captured bytes, tokens or cookies. Commits: Conventional Commits, ≤ 72-char subject, why-body, the user's git identity, no attribution trailers.

## Review Focus

1. **Content-Length chip with `Transfer-Encoding` present.** A pasted chunked request edited in assisted mode. Expected: the chip is skipped and recorded as `skipped: TE present`, bytes sent unchanged. Pinned in Task 4 (`assisted_skips_content_length_when_transfer_encoding_present`).
2. **Hold timeout with the browser still waiting.** A held request whose 120 s (configured 300 ms in test) expires. Expected: per setting, forwarded unchanged or answered with a generated 504 recorded as `Error`; the held item leaves the queue; the budget is released. Pinned in Task 5 (`hold_timeout_forwards_or_504s_per_setting`).
3. **`*.example.com` versus the apex and IP literals.** Expected: `api.example.com` in, `example.com` out unless `also_apex`, `93.184.216.34` out unless listed, `[::1]` out. Pinned in Task 2 (`suffix_rule_apex_and_ip_literals`).
4. **Cookie replay from another localhost port.** A page on `http://localhost:9999` fetches `/api/v1/exchanges` and opens the events WebSocket with the browser's cookie. Expected: 403 on both, nothing leaks. Pinned in Task 8 (`another_localhost_port_cannot_read_any_route_or_events`).
5. **Out-of-scope repeater send.** Under a non-empty scope: allowed with `scope_state: out` and one audit row; under `enforce_scope_strictly` refused with a typed error and audited; `Host` in scope but target out → `host-mismatch`. Pinned in Task 4 (`out_of_scope_send_is_audited_and_refused_in_strict_mode`).

---

### Task 0: Scaffold `crosswire-scope`, `crosswire-tools`, `crosswire-api`

**Files:**
- Create: `crates/crosswire-scope/{Cargo.toml,src/lib.rs}`, `crates/crosswire-tools/{Cargo.toml,src/lib.rs}`, `crates/crosswire-api/{Cargo.toml,src/lib.rs}`
- Modify: `Cargo.toml` (workspace deps), `deny.toml` (if a licence is new), `justfile` (`features` lines)

- [ ] **Step 1: Workspace deps**: `axum = { version = "0.8", default-features = false, features = ["http1", "json", "query", "tokio", "ws", "macros"] }`, `tower-http = { version = "0.6", features = ["trace"] }`, `utoipa = { version = "5", features = ["axum_extras", "ulid"] }`, `utoipa-axum = "0.2"`, `regex = "1"`, `ipnet = "2"`, `rand = "0.9"`, `subtle = "2"`, `http = "1"`, `url = "2"`.
- [ ] **Step 2: Manifests** in the sibling style (version `0.0.0`, `publish = false`, workspace lints). `crosswire-scope` depends on `crosswire-core`, `regex`, `ipnet`, `serde`, `thiserror`, `url`. `crosswire-tools` on `crosswire-{core,h1,model,client,scope}`, `tokio`, `bytes`, `thiserror`, `ulid`, `sha2`. `crosswire-api` on `crosswire-{model,store,scope,proxy,tools,ca}`, `axum`, `tower-http`, `utoipa`, `utoipa-axum`, `serde`, `serde_json`, `tokio`, `rand`, `sha2`, `subtle`, `base64`, `ulid`, `http`, `thiserror`, `tracing`. Each `lib.rs` carries the crate doc only.
- [ ] **Step 3: justfile**: add `cargo check --locked -p crosswire-scope --no-default-features` to `features`.
- [ ] **Step 4: Verify** `cargo build --workspace --locked`, `just lint` (via its commands), `just doc`, `cargo deny check`.
- [ ] **Step 5: Commit** `chore: scaffold scope, tools and api crates`.

---

### Task 1: h1 edit engine (`apply_edits`)

**Files:**
- Create: `crates/crosswire-h1/src/edit.rs`
- Modify: `crates/crosswire-h1/src/lib.rs` (exports), `crates/crosswire-h1/README.md`
- Test: `crates/crosswire-h1/tests/edit.rs`

**Interfaces:**
- Produces (spec §5.1, verbatim names): `pub struct Edit { pub span: Span, pub replacement: Bytes }`; `pub struct Edited { pub message: RawMessage, pub changes: Vec<Change>, pub new_diagnostics: Vec<Diagnostic> }`; `#[non_exhaustive] pub enum EditError { Overlap, OutOfRange, BodyNotInline, Reparse(ParseError) }`; `pub fn apply_edits(msg: &RawMessage, edits: &[Edit], cfg: &ParserConfig, reason: ChangeReason) -> Result<Edited, EditError>` (the config parameter is added so re-parsing uses the caller's profile and limits; the spec's signature omitted it). Spans address the concatenation `head ++ body` (body edits require an inline body); edits are sorted by `span.start`, must not overlap, and each produces one `Change` whose `span` is in after-coordinates with `before` = the replaced bytes and `after` = the replacement. `new_diagnostics` = diagnostics on the re-parsed message not present (by code + span) on the original.
- Also: `pub fn content_length_edit(msg: &RawMessage) -> Option<Edit>` returning the edit that sets `Content-Length` to the inline body length (None when there is no body, no inline body, when `Transfer-Encoding` is present, or when the header already matches); `pub fn crlf_edits(msg: &RawMessage) -> Vec<Edit>` turning bare LF line endings in the head into CRLF.

- [ ] **Step 1: Failing tests** in `tests/edit.rs`: `replace_one_header_keeps_every_body_byte` (body = all 256 byte values; edit the `User-Agent` value; assert `write_exact` of the result equals head' ++ body, body identical, one `Change{op: Replace, reason}` with correct after-span); `insert_header_at_end_of_head`; `delete_header_line`; `overlapping_edits_are_rejected`; `out_of_range_is_rejected`; `body_edit_without_inline_body_is_rejected`; `reparse_error_surfaces` (edit that removes the blank line); `content_length_edit_updates_and_skips_te` (a request with `Content-Length: 3` and a 5-byte body → edit to `5`; with `Transfer-Encoding: chunked` → None); `crlf_edits_normalise_bare_lf`; `new_diagnostics_reports_only_added_codes`.
- [ ] **Step 2: Run** → fail to compile.
- [ ] **Step 3: Implement** `edit.rs`; export from `lib.rs`; add an "Editing" section to the h1 README.
- [ ] **Step 4: Run** `cargo test -p crosswire-h1 --all-features`; `just features` lines for h1 still pass (no new deps; `Change` comes from core).
- [ ] **Step 5: Commit** `feat(h1): apply_edits with re-parse, Content-Length and CRLF edits`.

---

### Task 2: `crosswire-scope` grammar, evaluator, importer

**Files:**
- Create: `crates/crosswire-scope/src/{lib,rule,scope,import,input}.rs`, `crates/crosswire-scope/README.md`
- Test: `crates/crosswire-scope/tests/{matrix,import}.rs`

**Interfaces:**
- Produces: `pub enum HostPattern { Exact(String), Suffix { suffix: String, also_apex: bool }, Regex(String), Ip(IpAddr), Cidr(IpNet) }`; `pub enum PathMatch { Prefix(String), Regex(String) }`; `pub enum Scheme { Http, Https }`; `pub struct Rule { pub host: HostPattern, pub ports: Option<BTreeSet<u16>>, pub scheme: Option<Scheme>, pub path: Option<PathMatch> }` (serde, kebab-case `type` tags); `pub struct Scope { pub include: Vec<Rule>, pub exclude: Vec<Rule> }` (serde; `Scope::is_empty()`); `pub struct Compiled` built by `Scope::compile(&self) -> Result<Compiled, ScopeError>` (regexes compiled once with `RegexBuilder::size_limit(10 * 1024 * 1024)`; `ScopeError::{Regex(String), RegexTooLarge, BadCidr, BadPort}`); `pub struct Input<'a> { pub host: &'a str, pub port: u16, pub scheme: Scheme, pub path: &'a str }`; `pub enum Verdict { In, Out, Empty }`; `Compiled::evaluate(&self, input: &Input) -> Verdict`; `Compiled::evaluate_pair(&self, connection: &Input, message: &Input) -> ScopeState` (model's `ScopeState`: `In`/`Out`/`Empty`/`HostMismatch` when the two verdicts differ); `pub fn parse_program_list(text: &str) -> Result<Scope, ImportError>` accepting bare hosts, `*.wildcards`, full URLs (scheme, port, path prefix), CIDR, IP literals, `#` comments and blank lines, with `ImportError::Line { line: usize, reason: String }`.
- Hostnames compare ASCII-case-insensitively with a trailing dot stripped; IP literals match only `Ip`/`Cidr` rules; a bracketed IPv6 input is unbracketed before matching.

- [ ] **Step 1: Failing tests**: `tests/matrix.rs` is a table test `const CASES: &[(&str /*rule set*/, &str /*url*/, Verdict)]` of at least 40 rows covering: exact host; `*.example.com` with and without `also_apex` (Review Focus 3: `suffix_rule_apex_and_ip_literals`); regex host; IP literal vs name rule; CIDR `10.0.0.0/8` incl. `10.255.255.255` in and `11.0.0.0` out; port sets; scheme; path prefix and regex; exclude beats include; no include match → Out; empty scope → Empty; trailing dot and case; bracketed IPv6. `evaluate_pair` cases: both in → In; connection out/message in → HostMismatch; empty → Empty. `tests/import.rs`: a pasted list with every supported form → the expected `Scope`; a bad line → `ImportError::Line{line}`; regex over the size limit → `RegexTooLarge`.
- [ ] **Step 2: Run** → fail.
- [ ] **Step 3: Implement**; README with the grammar table and the precedence rule.
- [ ] **Step 4: Run** `cargo test -p crosswire-scope`; `cargo check -p crosswire-scope --no-default-features`.
- [ ] **Step 5: Commit** `feat(scope): rule grammar, evaluator and program-list importer`.

---

### Task 3: Store control path: settings, tokens, drafts, tags and notes

**Files:**
- Create: `crates/crosswire-store/migrations/0002_sidecar.sql`, `crates/crosswire-store/src/control.rs`
- Modify: `crates/crosswire-store/src/{schema,project,store,read,lib}.rs`, `docs/format.md`, `docs/adr/0002-store-durability.md` (one paragraph on the control connection)
- Test: `crates/crosswire-store/tests/control.rs`, `tests/project.rs` (migration)

**Interfaces:**
- Migration 2 (additive): `settings(key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at INTEGER NOT NULL)`, `tokens(id TEXT PRIMARY KEY, name TEXT NOT NULL, scopes TEXT NOT NULL, hash BLOB NOT NULL UNIQUE, created_at INTEGER NOT NULL, revoked_at INTEGER, last_used_at INTEGER)`, `drafts(tab_id TEXT PRIMARY KEY, json TEXT NOT NULL, updated_at INTEGER NOT NULL)`; `user_version` 2. `Project::open` migrates 1 → 2 after copying `project.db` to `project.db.v1.bak` (snapshot), refuses `user_version > 2` (older binary rule, exists).
- `Store::control(&self) -> Control` (a cheap handle; one dedicated write connection with `busy_timeout` 5 s, short transactions, WAL): `Control::setting_get(&self, key: &str) -> Result<Option<String>>`, `setting_put(&self, key, value: &str) -> Result<()>`; `token_create(&self, name: &str, scopes: &[TokenScope]) -> Result<(TokenRecord, String /*secret, shown once*/)>`, `token_list(&self) -> Result<Vec<TokenRecord>>`, `token_revoke(&self, id: Ulid) -> Result<bool>`, `token_lookup(&self, hash: &[u8; 32]) -> Result<Option<TokenRecord>>` (active only; updates `last_used_at` at most once per minute); `draft_put(&self, tab_id: Ulid, json: &str)`, `draft_get`, `draft_list`, `draft_delete`; `exchange_set_tags(&self, id: Ulid, tags: &[String]) -> Result<bool>`, `exchange_set_notes(&self, id: Ulid, notes: &str) -> Result<bool>` (update the `tags`/`notes` columns and the `json` document's fields in one transaction). `pub enum TokenScope { Read, Send, Admin }`; `pub struct TokenRecord { id, name, scopes: Vec<TokenScope>, created_at, revoked_at: Option<u64>, last_used_at: Option<u64> }`. The secret is 32 random bytes, base64url without padding, prefixed `cwt_`; `hash = sha256(secret bytes)`.
- Pre-0.1.0 backlog folded in: `Counts` gains `by_namespace`; `ExchangeFilter` gains `scope_state: Option<ScopeState>` and `tag: Option<String>` (exact match in the JSON tags array via `EXISTS (SELECT 1 FROM json_each(tags) WHERE value = ?)`).

- [ ] **Step 1: Failing tests**: `migrates_v1_project_to_v2_with_snapshot` (open a v1 fixture made by Plan 3's code path — create via `Project::create` then downgrade `user_version` and drop the tables in the test — assert `.v1.bak` exists and the tables appear); `settings_round_trip_and_overwrite`; `token_create_list_revoke_lookup` (secret starts with `cwt_`, lookup by hash finds it, revoked token is not found, listing never contains hashes or secrets); `drafts_round_trip`; `set_tags_and_notes_update_row_and_json` (then `Store::list(filter{tag})` finds it and `get()` shows the tags); `control_writes_do_not_block_the_writer_thread` (submit 1,000 exchanges while doing 100 settings writes; all commit; no SQLITE_BUSY surfaced).
- [ ] **Step 2: Run** → fail.
- [ ] **Step 3: Implement**; document the tables and migration in `docs/format.md`; one ADR-0002 paragraph: the control connection is a second writer for rare, small transactions and never touches `exchanges` bodies.
- [ ] **Step 4: Run** `cargo test -p crosswire-store --all-features` (incl. the kill -9 test).
- [ ] **Step 5: Commit** `feat(store): control connection with settings, tokens, drafts, tags and notes`.

---

### Task 4: `crosswire-tools` repeater

**Files:**
- Create: `crates/crosswire-tools/src/{lib,repeater,record,redirect}.rs`, `crates/crosswire-tools/README.md`
- Modify: `crates/crosswire-recorder-tests/src/lib.rs` (its `record_exchange` moves into `crosswire-tools::record`; the test crate re-exports it)
- Test: `crates/crosswire-tools/tests/repeater.rs` (raw/TLS upstream helpers copied from the proxy's `tests/common/upstream.rs` into `tests/common/`)

**Interfaces:**
- Consumes: `apply_edits`, `content_length_edit`, `crlf_edits` (Task 1); `Compiled::evaluate_pair` (Task 2); `Transport`, `Session`, `WritePlan`, `Target`, `VerifyMode` (client); `ExchangeSink`, `Namespace::new(NamespaceKind::Repeater, Some(tab))`, `ScopeState`, `Timing`.
- Produces: `pub enum Mode { Assisted, Raw }`; `pub enum FollowRedirects { Off, InScope, Always }`; `pub struct SendRequest { pub tab: Ulid, pub source: Option<Ulid>, pub host: String, pub port: u16, pub tls: bool, pub sni: Option<String>, pub verify: VerifyMode, pub raw: Bytes, pub mode: Mode, pub follow: FollowRedirects, pub reuse: bool, pub allow_out_of_scope: bool }`; `pub struct SendOutcome { pub exchange_id: Ulid, pub scope_state: ScopeState, pub changes: Vec<Change>, pub skipped: Vec<String> /* e.g. "content-length: skipped, TE present" */, pub cert_override: bool, pub ttfb_ms: u32, pub total_ms: u32, pub redirects: Vec<Ulid> }`; `#[non_exhaustive] pub enum SendError { OutOfScope { host: String }, Parse(String), Client(ClientError), Sink(SinkError), RedirectCap }`; `pub struct Repeater { transport: Arc<Transport>, sink: Arc<dyn ExchangeSink>, scope: Arc<RwLock<Compiled>>, strict: Arc<AtomicBool>, audit: Arc<dyn Fn(&str, &str, &str) + Send + Sync> /* action, target, result */, cfg: ParserConfig, sessions: Mutex<HashMap<(String,u16,bool), Session>> /* reuse */ }`; `Repeater::send(&self, req: SendRequest) -> Result<SendOutcome, SendError>`.
- Behaviour: parse `raw` with `Parser::request`; assisted mode applies `crlf_edits` then `content_length_edit` via `apply_edits(.., ChangeReason::ContentLength)` recording changes and skips; raw mode sends untouched with `ResponseContext::for_method` from a best-effort parse (until-close fallback); scope: evaluate the connection identity (`host`/SNI) and the message identity (`Host`) → `scope_state`; out-of-scope under a non-empty scope: refused with `SendError::OutOfScope` when `strict` or `!allow_out_of_scope`, else sent; every out-of-scope send audits `repeater.send.out_of_scope`; every send audits `repeater.send` with `host:port mode`. Record via `record::record_exchange` (moved recipe, extended to accept `Namespace`, `ScopeState`, diffs and an optional `source` link stored as a tag `source:<ulid>` for now); redirects: on 3xx with `Location`, follow per policy up to 10, each hop its own exchange with the same tab owner; `cert_override` and timing from the connection.

- [ ] **Step 1: Failing tests**: `raw_mode_sends_bytes_untouched` (body 0x00–0xFF; upstream receives identical bytes; result exchange in namespace repeater with owner = tab and tag `source:<id>`); `assisted_mode_fixes_content_length_and_crlf_and_records_changes`; `assisted_skips_content_length_when_transfer_encoding_present` (Review Focus 1); `out_of_scope_send_is_audited_and_refused_in_strict_mode` (Review Focus 5, incl. the host-mismatch case); `empty_scope_never_prompts_or_refuses`; `follow_redirects_in_scope_only_and_caps_at_10`; `reuse_keeps_one_upstream_connection`; `cert_override_surfaces_in_outcome` (upstream with an untrusted CA + `VerifyMode::AcceptInvalid`); `ttfb_and_total_are_populated`.
- [ ] **Step 2: Run** → fail.
- [ ] **Step 3: Implement**; move the recipe; README.
- [ ] **Step 4: Run** `cargo test -p crosswire-tools -p crosswire-recorder-tests --all-features`.
- [ ] **Step 5: Commit** `feat(tools): repeater with assisted and raw modes, scope checks and redirects`.

---

### Task 5: Request intercept in the proxy

**Files:**
- Create: `crates/crosswire-proxy/src/intercept.rs`
- Modify: `crates/crosswire-proxy/src/{handler,relay,listener,limits,lib}.rs`, `crates/crosswire-proxy/README.md`
- Test: `crates/crosswire-proxy/tests/intercept.rs`

**Interfaces:**
- `Handler` becomes async and body-aware (pre-0.1 breaking change inside the workspace): `pub trait Handler: Send + Sync { fn on_request<'a>(&'a self, req: HeldRequest<'a>) -> BoxFuture<'a, Decision>; fn on_response_head(&self, resp: &RawMessage, req: &RawMessage) -> Decision; }` with `pub struct HeldRequest<'a> { pub head: &'a RawMessage, pub body: Option<&'a Bytes> /* Some when the body fit under intercept_hold and was buffered */, pub conn: &'a ConnectionInfo, pub scope_state: ScopeState, pub editable: bool }` and `#[non_exhaustive] pub enum Decision { Forward, Drop, Modify { head: Bytes, body: Option<Bytes>, changes: Vec<Change> } }`. `NoopHandler` forwards. The relay calls `on_request` after the head (and, when `editable`, after buffering the body up to `intercept_hold`, charged to the budget); on `Modify` it writes the new bytes upstream and records them as the canonical request with the changes appended to the diffs (`ChangeReason::InterceptEdit`); `Drop` closes as today; `Forward` continues (with the buffered body written first).
- `pub struct Intercept` (the service, `Arc`-shared with the API): `Intercept::new(limits: &ProxyLimits, scope: Arc<RwLock<Compiled>>, rules: InterceptRules, budget: Arc<Budget>, sink: Arc<dyn ExchangeSink>)`; `enabled(&self) -> bool`, `set_enabled(&self, bool)` (audited by the caller); `rules(&self)/set_rules(&self, InterceptRules)`; `held(&self) -> Vec<HeldSummary { id: Ulid, host, method, path, size, editable, held_since }>`; `forward(&self, id: Ulid, edited: Option<EditedBytes { head: Bytes, body: Option<Bytes> }>) -> Result<(), InterceptError::{NotHeld, NotEditable, Edit(EditError)}>` (edited bytes are validated through `apply_edits`-style re-parse before release and the changes computed as one `Replace` of the whole head/body when a precise diff is unavailable); `drop(&self, id) -> Result<()>`; `forward_all(&self)`; `subscribe(&self) -> watch::Receiver<u64> /* held-count version */`. `pub struct InterceptRules { pub skip_extensions: Vec<String>, pub skip_out_of_scope: bool, pub overflow: Overflow::{ForwardNewest, ForwardOldest}, pub on_timeout: OnTimeout::{Forward, Respond504} }` with the Global-Constraints defaults. `Intercept` implements `Handler`: when disabled or a rule skips → `Forward` immediately; else enqueue (depth 64, overflow per setting), wait on a `oneshot` with `intercept_hold_timeout`; on timeout act per setting: `OnTimeout::Forward` → `Decision::Forward`, `OnTimeout::Respond504` → `Decision::Respond504`, a fourth variant the relay answers with its existing generated-504 helper (phase `"intercept hold"`) and records as `Error`. Final shape: `Decision::{Forward, Drop, Modify{..}, Respond504}`.
- `ProxyLimits` gains `intercept_hold: u64` (4 MiB), `intercept_hold_timeout: Duration` (120 s), `intercept_queue: usize` (64); `ProxyDeps.handler` is the `Arc<Intercept>` in the server; `ConnectionInfo`/scope: the relay evaluates the scope pair for every exchange (connection identity = `Dest.host` or SNI; message identity = `Host`) through a `Arc<RwLock<Compiled>>` on `ProxyDeps` (`scope`, default empty) and sets `Exchange::set_scope_state` — this also makes history filtering by scope work (Plan 5).

- [ ] **Step 1: Failing tests** (`tests/intercept.rs`, harness with the real `Intercept` as handler): `disabled_intercept_forwards_immediately`; `held_request_is_listed_and_forwarded_unchanged`; `held_request_forwarded_with_edited_head_records_changes` (upstream receives the edited bytes; the recorded request's diffs contain an `InterceptEdit` change); `held_request_dropped_closes_client`; `body_over_hold_limit_streams_through_not_editable` (5 MiB POST with `intercept_hold = 1 MiB`; `editable == false`; forwarding works; `forward(.., Some(edit))` → `NotEditable`); `hold_timeout_forwards_or_504s_per_setting` (Review Focus 2; both settings; the 504 is recorded as `Error`); `queue_overflow_forwards_per_setting` (depth 2; third hold → oldest or newest forwarded); `skip_rules_apply_extension_and_out_of_scope` (`.png` not held; out-of-scope host not held under a scope; held under an empty scope); `forward_all_releases_everything`; `scope_state_is_recorded_on_history_exchanges` (in / out / host-mismatch / empty).
- [ ] **Step 2: Run** → fail.
- [ ] **Step 3: Implement**; update the proxy README (intercept section, new limits); keep every existing proxy test green (`NoopHandler` callers unchanged in behaviour).
- [ ] **Step 4: Run** `cargo test -p crosswire-proxy --locked` three times.
- [ ] **Step 5: Commit** `feat(proxy): request intercept with hold, edit, drop and forward-all`.

---

### Task 6: `crosswire-api` REST and WebSocket with credentials

**Files:**
- Create: `crates/crosswire-api/src/{lib,app,auth,session,tokens,routes/mod,routes/exchanges,routes/scope,routes/intercept,routes/repeater,routes/tokens,routes/misc,events,error,openapi}.rs`, `crates/crosswire-api/README.md`
- Test: `crates/crosswire-api/tests/{auth,exchanges,scope,intercept,repeater,tokens,events}.rs` with `tests/common/mod.rs` (an in-process `App` over a temp `Store`, a `Repeater` with a raw upstream, an `Intercept`, and a `reqwest`-free client built on `hyper`/`axum::serve` + `tokio::net::TcpStream` raw HTTP — or add `reqwest` as a dev-dependency with `default-features = false, features = ["rustls-tls"]`; choose reqwest for brevity and note it in `deny.toml` if a licence is new)

**Interfaces:**
- Consumes: Tasks 2–5 types; `Store::{list,get,get_body,markers,counts,subscribe,control,audit}`; `Ca::{cert_der,cert_pem}`.
- Produces: `pub struct AppState { store: Arc<Store>, repeater: Arc<Repeater>, intercept: Arc<Intercept>, scope: Arc<RwLock<Compiled>>, scope_doc: Arc<RwLock<Scope>>, ca: Arc<Ca>, sessions: SessionStore, bootstrap: BootstrapStore, port: u16, strict: Arc<AtomicBool> }`; `pub fn router(state: AppState) -> axum::Router`; `pub fn bootstrap_token(state: &AppState) -> String` (mints the single-use token: 32 random bytes base64url, 60 s expiry); `pub struct ApiConfig { pub bind: SocketAddr, pub idle_timeout: Duration /* 30 min */ }`.
- Routes (all under `/api/v1`, JSON, utoipa-annotated, stable error body `{ "error": { "code": "<snake_case>", "message": "..." } }`):
  - Unauthenticated: `GET /health`, `GET /openapi.json`, `GET /ca.der`, `GET /ca.pem`, `POST /session` `{ "bootstrap": "<token>" }` → `Set-Cookie: cw_sid_<port>=...; HttpOnly; SameSite=Strict; Path=/api` and `{ "expires_in": 1800 }`; wrong/expired/reused token → 401 `bootstrap_invalid`, rate-limited 10/min per peer, audited `session.bootstrap` (ok | fail).
  - `read`: `GET /exchanges?limit&after&method&host&host_contains&status_min&status_max&namespace&state&scope_state&tag` → `Page<ExchangeRow>`; `GET /exchanges/{id}`; `GET /exchanges/{id}/body/{req|resp}` (`application/octet-stream`, `X-Crosswire-Length`, `X-Crosswire-Truncated-At` when partial); `GET /markers?after&limit`; `GET /counts`; `GET /scope`; `GET /intercept` (enabled, rules, held); `GET /repeater/tabs`, `GET /repeater/tabs/{id}`; `GET /tokens` (metadata only).
  - `send`: `POST /repeater/send` (`SendRequest` minus `tab` defaults) → `SendOutcome`; `PUT /repeater/tabs/{id}` (draft JSON ≤ 1 MiB); `DELETE /repeater/tabs/{id}`; `POST /intercept/held/{id}/forward` `{ head?: base64, body?: base64 }`; `POST /intercept/held/{id}/drop`; `POST /intercept/forward-all`; `PATCH /exchanges/{id}` `{ tags?, notes? }`.
  - `admin`: `PUT /scope` (`Scope` JSON; compiled before saving; audited `scope.set`); `POST /scope/import` `{ text }` → `Scope` (not saved); `PUT /intercept/enabled` `{ enabled }` (audited); `PUT /intercept/rules`; `PUT /settings/enforce_scope_strictly` `{ enabled }` (audited); `POST /tokens` `{ name, scopes }` → `{ token: TokenRecord, secret }` (audited `token.create`); `DELETE /tokens/{id}` (audited `token.revoke`).
  - `GET /events` WebSocket: cookie or `Authorization: Bearer` header or first text frame `{"bearer": "..."}` within 5 s; streams `Event` JSON from `Store::subscribe()` plus `{"type":"intercept","held":n}` on changes; `?after=<seq>` resyncs by replaying from the store's event buffer when available else sends `{"type":"lagged"}` first; per-subscriber buffer 10,000 then `lagged`.
  - Auth middleware order: Host check (421) → route allowlist → credential (cookie with same-site/Origin check, or bearer token hashed and looked up with constant-time compare of the hash) → scope check (403 `insufficient_scope`) → idle-expiry refresh. Never emits `Access-Control-*`. Session store in memory (sessions do not survive restart; tokens do).
- Logging: `tower_http::trace` at `debug` with method and path only.

- [ ] **Step 1: Failing tests**: `tests/auth.rs`: `bootstrap_is_single_use_and_expires` (60 s → use a 200 ms expiry via `ApiConfig` test hook); `bootstrap_rate_limited_after_10_failures_and_audited`; `cookie_without_sec_fetch_site_is_403`; `host_other_than_localhost_is_421`; `read_token_cannot_mutate` (every `send`/`admin` route → 403 for a read token); `revoked_token_is_401`; `no_cors_headers_ever`. `tests/exchanges.rs`: list with every filter, keyset paging, get, body with the truncation header, patch tags/notes. `tests/scope.rs`: put, get, import, compile error → 422 `scope_invalid`. `tests/intercept.rs`: enable, see a held item via the proxy harness, forward with an edit, drop. `tests/repeater.rs`: send raw and assisted, draft round trip. `tests/tokens.rs`: create shows the secret once, list hides it, revoke. `tests/events.rs`: receives `received`/`committed` for a submitted exchange; bearer first-frame auth; `after` resync; `lagged` under a slow consumer.
- [ ] **Step 2: Run** → fail.
- [ ] **Step 3: Implement**; generate `openapi.json` at build time into `docs/api/openapi.json` via a test (`openapi_snapshot_is_current` fails when the committed file differs; run with `UPDATE_OPENAPI=1` to rewrite) so the spec is versioned.
- [ ] **Step 4: Run** `cargo test -p crosswire-api`.
- [ ] **Step 5: Commit** `feat(api): REST and WebSocket API with bootstrap sessions and scoped tokens`.

---

### Task 7: Server composition: API listener, bootstrap, `token`, `browser`

**Files:**
- Create: `crates/crosswire-server/src/{api_cmd,browser,token_cmd}.rs`
- Modify: `crates/crosswire-server/src/{cli,serve,paths}.rs`, `crates/crosswire-server/README.md`
- Test: `crates/crosswire-server/tests/cli.rs` (extend), `tests/browser.rs`

**Interfaces:**
- `serve` gains `--api-bind ADDR` (default `127.0.0.1:8081`; non-loopback prints `warning: the API should stay on loopback` and binds anyway, per §11.2), `--bootstrap-file PATH` (write the bootstrap URL to a 0600 file instead of stdout), `--enforce-scope-strictly`; prints a third line `api listening on 127.0.0.1:8081` and a fourth `open http://localhost:8081/#bootstrap=<token> within 60 s` (the token appears on stdout exactly once and never in logs); the scope and intercept rules are loaded from `settings` at open (intercept always starts disabled); the proxy's `ProxyDeps.handler` is the `Intercept`, `ProxyDeps.scope` the shared compiled scope.
- `crosswire token create --project DIR --name NAME --scopes read,send` (prints the secret once; audited), `token list`, `token revoke ID`, `token rotate ID` (revoke + create with the same name/scopes, prints what it invalidated).
- `crosswire browser --project DIR [--browser-path PATH] [--proxy ADDR] [--headless] [--url URL] [-- extra flags]`: detection order Chrome → Chromium → Edge → Brave per OS (macOS app bundles, Linux `PATH` names, Windows `Program Files`), or `--browser-path`; flags exactly `--proxy-server=<proxy> --proxy-bypass-list=<-loopback> --user-data-dir=<project>/browser-profile --ignore-certificate-errors-spki-list=<CA SPKI> --no-first-run --no-default-browser-check` plus pass-through; the profile dir is created 0700 under the project (and listed in `docs/format.md` as excluded from exports); `crosswire browser reset-profile --project DIR` deletes it (audited `browser.profile_reset`); when no browser is found: prints the proxy address and `http://crosswire/` and exits 3. The launcher reads the proxy address from `--proxy` or the project's `settings.proxy_bind` written by `serve` at start.
- `paths.rs`: `bootstrap_default_dir()` unused (kept simple: stdout or `--bootstrap-file`).

- [ ] **Step 1: Failing tests**: `serve_prints_api_line_and_bootstrap_once_and_session_works` (spawn, parse the four lines, `POST /api/v1/session` with the token → cookie; second use → 401; `GET /api/v1/counts` with the cookie and `Sec-Fetch-Site: same-origin` → 200); `bootstrap_file_is_0600_and_not_printed`; `token_create_list_revoke_cli` (secret printed once; `list` shows no secret; a revoked token → 401 via the API); `browser_launches_headless_and_session_is_recorded` (`crosswire browser --headless --url https://browser.test:<port>/` against a running `serve` with `--allow` and `resolve` hooks — since `serve` has no resolve override, use a TLS upstream reachable as `127.0.0.1:<port>` and `--accept-invalid-cert 127.0.0.1`; assert the exchange via the API; `#[ignore]` unless `CROSSWIRE_BROWSER` is set, as in Plan 3); `browser_without_any_browser_prints_fallback_and_exits_3` (`--browser-path /nonexistent`).
- [ ] **Step 2: Run** → fail.
- [ ] **Step 3: Implement**; README sections for the API, tokens and the launcher; `justfile` `browser-test` also runs the server's browser test.
- [ ] **Step 4: Run** `cargo test -p crosswire-server --locked` (three runs), the ignored browser test locally with Chrome.
- [ ] **Step 5: Commit** `feat(server): API listener with bootstrap, token subcommands, crosswire browser launcher`.

---

### Task 8: Security gates without a UI (§13)

**Files:**
- Create: `crates/crosswire-api/tests/gates.rs`, `crates/crosswire-server/tests/gates.rs`
- Modify: `.github/workflows/ci.yml` (no new job; these run under `just test`), `docs/security-gates.md` (new: the §13 list with the test that proves each, and which wait for the UI)

- [ ] **Step 1: Write the gates as tests**, each named after the §13 item: `route_auth_every_route_except_the_five_is_401` (walk the OpenAPI document's paths × methods; assert 401 without credentials except the allowlist); `read_token_cannot_mutate` (walk every non-GET route with a read token → 403); `another_localhost_port_cannot_read_any_route_or_events` (Review Focus 4: a request whose `Origin` is `http://localhost:9999` and no `Sec-Fetch-Site`, cookie attached → 403 on every route and on the WebSocket upgrade); `host_421`; `bootstrap_single_use`; `non_loopback_proxy_requires_auth` (exists in the proxy; reference it); `loopback_spelling_relay_refusal` (exists; reference); `proxy_origin_form_refusal` (exists; reference); `scope_matrix_including_empty_scope` (history `scope_state` for in/out/host-mismatch/empty via the proxy harness + the repeater's four cases); `idle_egress_zero_connections` (start `serve` with a counting `Transport` wrapper — add a `#[doc(hidden)]` connect counter on `Transport` — wait 10 s idle with the API open and the events stream connected; assert 0 upstream connects and 0 DNS lookups); `ca_key_never_leaves` (every API response body, every file under the project dir, and the OpenAPI document contain neither the CA private key PEM nor its DER bytes); `audit_completeness` (perform every action in the Global Constraints list through the API/CLI and assert one row each in `audit.jsonl` with the documented action names); `auth_failures_rate_limited_and_audited`.
- [ ] **Step 2: Run** → fix anything a gate finds (each fix is its own `fix(...)` commit naming the gate).
- [ ] **Step 3: `docs/security-gates.md`**: the table; UI-dependent gates (XSS corpus, preview zero-egress, redaction golden for copy-as-curl) marked "Plan 5/6".
- [ ] **Step 4: Commit** `test: §13 security gates provable without a UI`.

---

### Task 9: Install-to-first-capture script and the alpha.2 mini-DoD

**Files:**
- Create: `scripts/first-capture.sh`, `crates/crosswire-server/tests/first_capture.rs`
- Modify: `.github/workflows/ci.yml` (ubuntu step after the browser test: `just first-capture`), `justfile`

- [ ] **Step 1: `scripts/first-capture.sh`**: builds the binary (`cargo build --release -p crosswire-server`), starts `serve --project $TMP --api-bind 127.0.0.1:0 --proxy-bind 127.0.0.1:0 --bootstrap-file $TMP/boot`, launches `crosswire browser --headless --url https://example.com/` (real egress is allowed only in this script, never in tests; CI has network), polls `GET /api/v1/exchanges?host=example.com` with the bootstrap session until one row appears, prints the elapsed seconds, fails over 120 s. The Rust test `first_capture_under_two_minutes` wraps the same flow against the in-process TLS upstream (no egress) and asserts < 120 s; the script measures the real thing.
- [ ] **Step 2: Run** locally with Chrome; record the time in the report.
- [ ] **Step 3: CI**: `just first-capture` on ubuntu only (`if: matrix.os == 'ubuntu-latest'`), after `just browser-test`.
- [ ] **Step 4: Commit** `test(server): install-to-first-capture script and timing test`.

The alpha.2 mini-DoD's third item (RSS < 500 MB at 1M synthetic exchanges) needs the history view's queries and is Plan 6 (performance), together with the §9 index work; state that in the docs task.

---

### Task 10: Docs, ADR-0004, format, changelog

**Files:**
- Create: `docs/adr/0004-api-credentials.md` (bootstrap → cookie, scoped tokens hashed at rest, same-site enforcement by `Sec-Fetch-Site`/`Origin`, why no CORS, why sessions are in memory; alternatives: PAKE/localhost-only trust rejected), `docs/api.md` (how to call the API with curl: bootstrap, tokens, events; links `docs/api/openapi.json`)
- Modify: `crates/crosswire-{scope,tools,api}/README.md` (verify), `crates/crosswire-{proxy,store,server,h1}/README.md` (new sections), `docs/format.md` (migration 2, profile dir, tokens), `CHANGELOG.md` (Unreleased: alpha.2 backend), `docs/log.md` (rows "sidecar 0".."sidecar 10", `n/m`; tokei rerun), root `README.md` (status: alpha.2 backend without UI; the API quick start; roadmap state), spec `§17` (dated paragraph: built — edit engine, scope, intercept (request), repeater, API, launcher; still deferred — response intercept (cut list), OS keychain, pack-writer streaming, 1M RSS (Plan 6), web UI (Plan 5)), `docs/security-gates.md` (verify).
- [ ] **Step 1: Write** with every claim checked against code; link check loop.
- [ ] **Step 2: Verify** `just doc`, `cargo test --workspace --locked --all-features`, `cargo deny check`, `just lint`.
- [ ] **Step 3: Commit** `docs(sidecar): API guide, ADR-0004 credentials, format and changelog`.

---

## Self-review notes

- **Spec coverage:** §5.1 edit engine → T1; §4.6/4.7 scope meanings, model, import → T2 (+T4/T5 for the active-send and intercept meanings, T6 for history filters); §7.2 repeater backend (modes, chips, redirects, connection settings, drafts, namespace) → T4/T6; the editor byte model and paste-raw flows are UI (Plan 5); §7.3 request intercept (rules, actions, hold limits, overflow/timeout) → T5/T6; response intercept is on the §14 cut list — deferred; §7.6 scope import → T2/T6, `crosswire browser` → T7; §8 REST + WS with seq ids, lagged, resync, keyset paging, OpenAPI → T6; §9.4 audited actions → T3–T7 + the T8 completeness gate; §11.2 credentials, cookie isolation, host checks, route-auth → T6/T8; §12 launcher and install-to-first-capture → T7/T9; §13 gates provable without UI → T8; §14 alpha.2 mini-DoD: gates (T8), first capture (T9), 1M RSS → Plan 6. **Deferred and named in §17 by T10:** response intercept, OS keychain, UI (Plan 5), 1M RSS + indexes (Plan 6), MCP/CLI (v0.2b).
- **Type consistency:** `Compiled`, `Scope`, `Input`, `Verdict` (T2) used in T4, T5, T6; `apply_edits`/`Edit`/`Change` (T1) in T4, T5; `Control`, `TokenScope`, `TokenRecord` (T3) in T6, T7; `Repeater`, `SendRequest`, `SendOutcome` (T4) in T6; `Intercept`, `HeldSummary`, `InterceptRules`, `Decision::{Forward,Drop,Modify,Respond504}` (T5) in T6, T7; `AppState`, `router`, `bootstrap_token`, `ApiConfig` (T6) in T7, T8, T9.
- **Review Focus:** each of the five has a named test (T4 ×2, T5, T2, T8).
- **Proportion:** signatures, routes, test names and assertions only; no bodies.
- **Backlog items folded in:** `ExchangeFilter` scope_state/tag and `Counts.by_namespace` (T3); scope evaluation recorded on history exchanges (T5); the `Transport` connect counter for the idle-egress gate (T8). Not folded (still backlog): schema-freeze D-list, gate address ranges, inline-body redaction on export, store bounds.
