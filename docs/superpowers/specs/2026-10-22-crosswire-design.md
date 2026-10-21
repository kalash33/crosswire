# Crosswire: design spec (v2)

- **Date:** 2026-10-22
- **Status:** revision 2, after the first judge panel (average 5.2/10, every reviewer "pass with fixes"). This revision applies all 16 MUST changes and most SHOULD changes from the chair's list.
- **Working name:** Crosswire. The crates.io names are free. "CrossWire" (a Bible-software project) exists, so the name needs a GitHub org, domain and trademark check before launch (§16).

## 1. What it is

Crosswire is an open-source web application security testing platform for authorized testing: an intercepting proxy, request history, repeater, and later intruder, session rules, multi-role testing, team mode, plugins and a scanner.

**Product pitch (outcomes, not language):**
- Open source and free in full. Nothing is gated.
- Sends exactly the bytes you wrote.
- Memory stays bounded at 1M+ requests.
- Projects can't corrupt.
- Free self-hosted team mode (v0.4).

**Contributor and library pitch:** written in Rust, on top of `crosswire-wire`, a sans-IO, byte-exact HTTP library for security tooling. It accepts malformed input, emits exactly what it is asked to on output, exposes byte spans, and reports diagnostics. The Rust ecosystem has no such library.

**Sustainability:** pure community open source. No paid tiers, no closed parts. Funding never gates the roadmap.

**Builder:** one part-time maintainer (about 12 hours/week) pairing with an AI assistant. Co-maintainers are recruited from v0.2.

## 2. Why it exists

- **Burp Suite Pro:** $499/user/year plus expiring AI credits; about 88k customers. Users repeatedly complain about memory use, corrupted or multi-GB project files, no native collaboration and a dated UI.
- **Caido:** closed-source, Rust, $200/year. About 36k new users in 2025 (people do switch). Free tier: 2 projects, 3 plugins; team mode paid; outbound HTTP/2 incomplete (issue #439 open since 2023); scanner is a community plugin.
- **ZAP:** Java, dated UX, roadmap influenced by Checkmarx. **Hetty:** 12.5k stars, no release since 2022. **mitmproxy:** a developer tool. **OpenTRAP:** launched 2026-10-19, a Java UI on ZAP.

### 2.1 Prior art for the wire layer

| Project | What it does | Why it is not enough |
|---|---|---|
| `httparse` | Zero-copy HTTP/1 head parser | Strict by design, no writer, no body framing |
| `hyper` | Production HTTP client/server | Normalises framing and headers and refuses invalid messages, by design |
| `h2` | HTTP/2 | Enforces the spec; cannot emit non-compliant frames or header blocks |
| `rama`, `hudsucker`, `http-mitm-proxy` | Proxy frameworks | Built on hyper, so they inherit its normalisation |
| `proxelar` | Rust MITM workbench, own stack | Developer tool; stack not published as a library |
| `tungstenite` | WebSocket | Compliant only |

**`crosswire-wire` charter:** tolerant parsing with diagnostics; exact writing; byte spans for every element; sans-IO. **Non-goal:** a general-purpose HTTP client.

### 2.2 Competitive matrix (honest)

| | Burp Pro | Caido paid | Caido free | ZAP | mitmproxy | **Crosswire v0.1** | **Crosswire v0.4** |
|---|---|---|---|---|---|---|---|
| Open source | no | no | no | yes | yes | **yes** | **yes** |
| Price | $499/yr | $200/yr | free | free | free | **free** | **free** |
| Exact-bytes HTTP/1 repeater | partial | partial | partial | no | partial | **yes** | **yes** |
| HTTP/2 | yes | beta | beta | yes | yes | no (1.1 only) | yes |
| Scanner | best | plugin | plugin | yes | no | no | passive only |
| Extensions | ~300 | ~49 | 3 max | many | addons | no | JS plugins |
| Team mode | no | paid | no | no | no | no | **free** |
| Bounded memory at 1M requests | no | no | — | no | no | **target** | **target** |

Where Crosswire loses for a long time: scanner depth and the extension ecosystem.

## 3. Users, and the release that serves each

1. **Bug bounty hunters** (v0.1–v0.2): free, fast, exact repeater, never loses work.
2. **Small pentest firms, 3–30 testers** (v0.4): free team mode, reporting.
3. **Students, CTF players, price-sensitive regions** (v0.1): free with no limits.
4. **AppSec engineers** (v0.2): CLI, headless, read-only MCP.
5. **Large consultancies** (after v1.0): procurement keeps them on Burp until then.

## 4. Product principles

1. **Fidelity means no hidden changes.** What is stored and sent is exactly what the fidelity contract (§6) promises. Conveniences (e.g. updating Content-Length) are visible, can be toggled off, and show when they will change bytes.
2. **Never lose work.** Crash-safe storage with a stated durability window (§9). Projects are directories, not single files. History is an append-only log plus audited purge.
3. **Bounded resources.** Memory does not grow with history size; every hostile-input dimension has a limit (§11.6).
4. **API-first.** UI, CLI, MCP and plugins are all clients of one versioned API.
5. **Free means everything.**
6. **Safe by default** (§11). Scope has three separate meanings:
   - **Proxy:** out-of-scope traffic passes through, is not intercepted, and is not logged by default (configurable). Blocking it is opt-in.
   - **Active tools** (repeater, intruder, CLI send, MCP): out-of-scope requests are never sent. A per-run override is allowed, is written to the audit log, and is never available over MCP.
   - **History:** in-scope only by default.
   - **Empty scope:** the proxy records everything; active tools ask the user to set a scope.
7. **Library quality.** Published crates are sans-IO where possible, `#![forbid(unsafe_code)]`, fuzzed, semver-checked and documented.
8. **No phoning home.** No telemetry; no outbound calls while idle (tested in CI). Update checks and crash reports are opt-in.

## 5. Architecture

```
clients:  web UI (Svelte/TS)    CLI (v0.2)    MCP (v0.2)    plugins (v0.3)
                      │   REST /api/v1 + WS events (OpenAPI → generated TS/Py clients)
                      ▼
┌──────────────── crosswire-server (binary) ────────────────────┐
│  API listener (loopback only)        Proxy listener           │
│              │                            │                   │
│        crosswire-tools  ──────────►  crosswire-proxy          │
│      (repeater, M&R, intruder)      (MITM engine, Handler)    │
│              │                            │                   │
│              ▼                            ▼                   │
│        crosswire-store  ◄── writer thread ◄── bounded queue   │
│     (SQLite WAL + pack files)                                 │
│              ▲                                                │
│        crosswire-query (parse/AST/eval; SQL lowering in store)│
│                                                               │
│  crosswire-client (tokio I/O, TlsBackend, Connection, WritePlan)
│      ├─ crosswire-wire    sans-IO HTTP/1, exact      (v0.1)   │
│      ├─ crosswire-h2wire  sans-IO HTTP/2 + HPACK     (v0.3)   │
│      └─ crosswire-ws      sans-IO WebSocket          (v0.3)   │
│  crosswire-ca (CA, leaf certs)      crosswire-model           │
└───────────────────────────────────────────────────────────────┘
```

| Crate | Published | Responsibility | Est. size |
|---|---|---|---|
| `crosswire-wire` | yes, 0.1 once proxy + repeater use it | Sans-IO; depends only on `bytes`. `RawMessage`, streaming tolerant HTTP/1 parser (state machine), exact writer, diagnostics, parse profiles. | ~4k LoC |
| `crosswire-h2wire` | later (v0.3) | Sans-IO HTTP/2 frame codec + HPACK; can emit non-compliant output on request. | ~5k |
| `crosswire-ws` | later (v0.3) | Sans-IO WebSocket frame codec. | ~1k |
| `crosswire-client` | later (v0.2+) | tokio I/O, `TlsBackend` trait (rustls default), explicit `Connection`, `WritePlan` staged writes, upstream HTTP/SOCKS5 chaining. Pooling is a `pool` feature, never implicit. | ~3k |
| `crosswire-ca` | yes | Local CA, keychain storage, leaf certs + cache, exports, browser-profile trust. | ~1k |
| `crosswire-model` | no (until 1.0) | Exchange, ULID ids, events, scope, fidelity labels. | ~1k |
| `crosswire-query` | no (until 1.0) | Filter language: parser, AST, type-checker, `FieldRegistry`, in-memory evaluator. | ~2k |
| `crosswire-store` | no | Project storage; owns lowering of queries to parameterised SQL. | ~3k |
| `crosswire-proxy` | no (until a stable `Handler` trait) | MITM engine. | ~3k |
| `crosswire-tools` | no | Repeater, match-and-replace, decoder, intruder (v0.2). | ~2k |
| `crosswire-api` | no | REST + WS handlers, OpenAPI via `utoipa`. | ~2k |
| `crosswire-server` | binary | Composition, listeners, auth, embedded UI. | ~1k |
| `crosswire-cli` | binary (v0.2) | Headless commands. | ~1k |
| `ui/` | — | Svelte + TypeScript; virtualised tables; CodeMirror 6. | ~8k TS |

**Why REST + OpenAPI (not GraphQL):** generated typed clients, a clean mapping to MCP, trivial curl/CI use, one familiar style for contributors.

**Why our own wire layer:** §2.1. Mature crates are still used where fidelity is not at stake: `tokio`, `rustls`, `rcgen`, `rusqlite`, `zstd`, later `quinn`.

### 5.1 `crosswire-wire` API sketch

```rust
pub struct RawMessage { buf: Bytes, view: HeadView, diagnostics: Vec<Diagnostic> }
pub struct HeadView { start_line: Span, headers: Vec<HeaderSpan>, body: BodyFraming }
pub struct HeaderSpan { name: Span, value: Span, line: Span }      // spans index into buf
#[non_exhaustive] pub enum DiagnosticCode { DuplicateContentLength, ConflictingFraming, ObsFold, BareLf, /* … */ }
pub enum BodyFraming { None, ContentLength(u64), Chunked, UntilClose, Ambiguous(Vec<BodyFraming>) }
pub struct Parser { /* streaming; feed(&[u8]) -> Progress */ }
pub fn write_exact(msg: &RawMessage, out: &mut BytesMut);          // always byte-identical
impl RawMessage { pub fn to_http_lossy(&self) -> http::Request<()> } // documented lossy
pub enum Profile { Strict, Lenient /* default */ }                  // named server emulations later
```

In `crosswire-client`, `Connection::write_plan(WritePlan)` supports staged writes (send N bytes, hold, release) so timing-sensitive sends can be added later without an API break. `send_request()` is a thin helper on top.

## 6. Fidelity contract

**HTTP/1.x (v0.1).** The canonical record is the raw byte stream in each direction per exchange. The proxy forwards bytes unchanged except for a documented list: hop-by-hop headers named in `Connection`, `Proxy-*` headers, and rewriting an absolute-form target to origin-form. Each change is recorded as a diff on the exchange.

**Derived views** (dechunked, decompressed, pretty-printed) are generated on demand, labelled derived, and never stored as the original.

**HTTP/2 (model fixed now; code v0.2/v0.3).** The canonical record is the ordered frame sequence per stream (type, flags, stream id, payload), plus the decoded header list with per-field encoding hints (indexing mode, Huffman, name casing, duplicates, pseudo-header order). Dynamic-table state is recorded per connection. "Exact replay" means re-encoding the edited header list and hints against a fresh or chosen table state, or, in expert mode, sending raw header-block bytes on a fresh connection.

**Fidelity label on every exchange:** `raw` (own codec, exact), `decoded-ordered` (frames and hints kept), `normalized` (came through a compliant library). The UI and API always show it.

**Protocol support matrix**

| Release | Browser ↔ proxy | Proxy ↔ upstream | Fidelity |
|---|---|---|---|
| v0.1 | HTTP/1.1 only (ALPN offers http/1.1); WS/101 tunnelled; TLS passthrough by SNI list; raw tunnel fallback | HTTP/1.1 | raw |
| v0.2 | HTTP/1.1 | HTTP/1.1; HTTP/2 via `h2` behind an adapter | raw / normalized |
| v0.3 | HTTP/1.1 | HTTP/2 via `crosswire-h2wire` (client role) | raw / decoded-ordered |
| v0.4 | HTTP/1.1 + HTTP/2 (server role, same codec) | HTTP/1.1 + HTTP/2 | raw / decoded-ordered |
| v0.5+ | adds HTTP/3 | adds HTTP/3 (`quinn`) | decided by ADR then |

**Golden tests:** HTTP/1 captures round-trip byte for byte; HTTP/2 captures round-trip frame for frame per connection.

## 7. Behaviour

### 7.1 Proxying
1. Browser sends CONNECT.
2. SNI is checked against the passthrough list; listed hosts are tunnelled raw with metadata-only logging.
3. Otherwise TLS is terminated with a cached leaf cert. On handshake failure: raw tunnel fallback plus an error event.
4. `wire` parses incrementally; scope and intercept rules apply; the request is forwarded through `crosswire-client`; the response is relayed.
5. The exchange is queued to the store (§9) and an event is broadcast.

**Body model:** head = owned bytes + spans. Body is inline up to 1 MiB, otherwise streamed: relayed while copied to the blob writer with a bounded window. A capture cap keeps the first N MB plus full hash and length; truncation is shown.

**WebSocket in v0.1:** after a 101, bytes are relayed both ways unparsed; the upgrade exchange is logged. Frame history, editing and replay: v0.3.

### 7.2 Repeater (v0.1)
Per-tab raw editor. Toggles (all off in raw mode; the UI shows when one will change bytes): update Content-Length (on by default outside raw mode), re-chunk, normalise line endings, follow redirects (off / in-scope / always). Connection settings: target host, port, TLS and SNI separate from the Host header; fresh vs reused connection. Results live in a `repeater` namespace linked to the source exchange, purgeable per tab.

### 7.3 Intercept (v0.1)
Requests and responses. Rules: simple field filters in v0.1, full query language in v0.2. Actions: forward, edit-and-forward, drop, forward-all, send-to-repeater. Heads are held with bodies up to a cap; larger bodies stream through marked "not editable". The hold queue is bounded; on overflow the configurable timeout action (forward or 504) applies. HTTP/2 holds (v0.3+) are per stream while PING/WINDOW_UPDATE keep flowing; timeout → RST_STREAM or 504.

### 7.4 Match-and-replace (v0.1)
Ordered rules on request/response head or body, literal or regex, applied in the proxy path and recorded in the exchange diff. `${var}` placeholders are reserved from v0.1 so v0.3 session rules (extract a value by regex/JSONPath → variable → inject; login macro on 401/302) need no rewrite.

### 7.5 Intruder (v0.2)
Positions are byte ranges in the raw request; length fields adjust only when the Content-Length toggle is on. Attack types: sniper, battering ram, pitchfork, cluster bomb. Bounded concurrency, rate limit, pause/stop. Results in a per-attack namespace with purge.

### 7.6 Small daily tools (v0.1)
Inspector/decoder pane (URL, base64, hex, JWT view, HTML entities); tags and notes; copy-as-curl; `crosswire browser` launches Chromium or Firefox with its own profile, the proxy set, and the CA trusted only in that profile.

## 8. API and events
- REST under `/api/v1`, OpenAPI-described; N-1 deprecation after a bump; stable error codes.
- Keyset (cursor) pagination only.
- WebSocket event stream with monotonic sequence ids; bounded per-subscriber buffer; a slow client receives `lagged` and resyncs from its cursor; events are filtered per subscriber (team-ready).

## 9. Storage
- **Layout:** a project is a directory (0700): `project.db` (SQLite WAL, `synchronous=NORMAL`, `application_id`, `user_version` = schema version), `packs/` (append-only zstd pack files), `journal/` (spill).
- **Bodies:** < 16 KiB inline in SQLite; larger in pack files, SHA-256 over raw bytes, deduplicated. A pack segment is fsynced before the metadata row referencing it commits. Orphan sweep at startup.
- **Writer:** one OS thread drains a bounded mpsc queue and commits batches every 250 ms or 1,000 exchanges, whichever first (the stated durability window). Queue full → spill to the on-disk journal. Journal fails (disk full) → **the proxy keeps relaying and stops recording**, with a persistent alert and a gap marker. Traffic is never blocked.
- **Maintenance:** periodic WAL checkpoints; per-project size/age quota; audited purge that also clears FTS rows and GCs blobs.
- **Search (v0.2):** FTS5, external content, trigram tokenizer, built asynchronously and capped per body; Authorization/Cookie/Set-Cookie excluded; `~` = trigram MATCH + REGEXP recheck.
- **Foreign projects:** SQLite defensive mode, `trusted_schema=OFF`.
- **Format versioning:** `docs/format.md`; snapshot before migration; an older binary refuses a newer project.

## 10. Filter language
- **v0.1:** simple field filters (method, host, path, status, MIME, size, tags, in-scope) in UI and API.
- **v0.2:** full language, e.g. `req.method == "POST" and resp.status >= 500 and req.host ~ "api."`. Parse → AST → type-check against `FieldRegistry`; evaluate in memory (intercept rules, live filters) or lower in `crosswire-store` to parameterised SQL only. Grammar documented; parser fuzzed.

## 11. Security of the tool itself

### 11.0 Threat model

| Asset | Adversary | Controls |
|---|---|---|
| API token (full control) | hostile target content rendered in the UI; hostile web page; local user | untrusted-content rendering (§11.1), listener split and token handling (§11.2), route-auth test |
| CA private key | local user; malware; team server | keychain or 0600 file; never in API, export or sync (§11.3) |
| Captured credentials and PII | local user; exports; plugins; teammates | 0700/0600 permissions; redacted exports; optional encryption at rest (v0.2); plugin capability gating (§11.5) |
| The proxy process | hostile servers and clients | resource limits (§11.6); `forbid(unsafe_code)` in sans-IO crates |
| Team server (v0.4) | a teammate; the network | per-project roles, hashed tokens, filtered events, audit log, egress denylist, CA never on the server |
| Users and the build | supply chain | §11.7 |

Trust boundaries: browser ↔ proxy listener; UI/CLI/MCP ↔ API listener; server ↔ upstream; server ↔ plugins (v0.3); client ↔ team server (v0.4). Every feature updates this table before it merges.

### 11.1 Untrusted content rendering
Captured bytes are rendered as text only (CodeMirror or text nodes); `{@html}` is banned by a CI lint. HTML/image previews render in a sandboxed iframe without `allow-same-origin`, served from a separate origin with `Content-Security-Policy: sandbox`. The UI is served with a strict CSP (`default-src 'self'`, no inline or eval), `X-Content-Type-Options: nosniff`, `frame-ancestors 'none'`. The CLI strips ANSI/terminal control sequences from captured content. Playwright tests load an XSS corpus into history, repeater and intercept and assert nothing executes. XML imports disable DTDs; importers are fuzzed.

### 11.2 Listeners and tokens
The API/UI listener and the proxy listener are separate sockets. The API listener binds to loopback only. The proxy may bind elsewhere (mobile testing) only with a flag, a printed warning and optional proxy authentication; it refuses to forward to its own API port. The token is stored in a 0600 file; the UI receives it once in the URL fragment and exchanges it for an HttpOnly, SameSite=Strict cookie; it is never sent in a query string. WebSocket upgrades are authenticated and checked against an Origin allowlist. A test enumerates every route and asserts auth is required. `crosswire token rotate` exists.

### 11.3 CA handling
Key in the OS keychain by default, 0600 file fallback. Trusted only in the `crosswire browser` profile by default; OS-wide trust is opt-in. Unique subject per install, one-year validity. One P-256 leaf keypair per install; leaf certs cached on disk. `ca rotate`, `ca uninstall`. DER and PEM export with Android and iOS instructions. A test asserts the private key never appears in any API response, export or sync.

### 11.4 Sensitive data
Directories 0700, files 0600. Optional encryption at rest (v0.2). A purge command that clears FTS, WAL and blobs. Exports redact Authorization, Cookie and Set-Cookie by default. Structured logs redact secrets.

### 11.5 Plugins (v0.3)
Separate worker process with memory and CPU limits. Network access limited to declared hosts in scope. Store reads are redacted unless a "secrets" capability is granted. UI plugins run in a sandboxed iframe. Users are re-prompted when a plugin's capabilities grow. Crash isolation is per process, not per task.

### 11.6 Resource limits against hostile peers
Configurable limits on head size, header count and line length; HPACK table size and CONTINUATION total; concurrent streams; decompressed size and ratio; per-exchange timeouts; disk quota per project. On a limit: emit a diagnostic and keep raw bytes only. A hostile-server fixture per limit lives in the test suite.

### 11.7 Supply chain and governance
SECURITY.md with private reporting via GitHub Security Advisories and a 90-day window. DCO sign-off. MSRV = stable-2, tested in CI. cargo-semver-checks, cargo-deny, cargo-audit in CI. Actions pinned by SHA; `npm ci` with a lockfile. Signed releases (Sigstore) with SLSA provenance. crates.io trusted publishing with 2FA. Signed Tauri updater (v0.5). Per-crate changelog via release-plz. AI-assisted PR policy: tests required, the author must be able to explain the change, wire changes need golden tests.

### 11.8 MCP (v0.2)
Its own token; read-only by default; sends allowed in scope only. Never exposes scope override, export, plugin install or token operations. Target-derived content is labelled untrusted data. Rate-limited and audit-logged.

## 12. Configuration, platforms, distribution
- **Config:** per-OS paths, precedence (flags > env > project > user > defaults), per-project overrides, listeners, first-run token.
- **Platforms:** tier 1 = Linux x86_64/aarch64, macOS aarch64; tier 2 = macOS x86_64, Windows x86_64 (CI nightly and non-blocking until v0.2).
- **Distribution:** single static binary with the UI embedded; `cargo binstall`; Homebrew tap; Docker image. CA install helpers incl. Firefox NSS and Android. Target: under 2 minutes from install to first intercepted HTTPS request.
- **Updates:** opt-in update check; `/api/v1` N-1 deprecation; project-format migrations with snapshot.

## 13. Testing and performance
- **Golden fidelity tests** (§6). **Fuzzing** (`cargo-fuzz`): HTTP/1 parser, HPACK/frames (v0.3), query parser, importers; smoke in CI, full nightly. **Property tests** (`proptest`) for parse/write round-trips.
- **Integration harness:** in-process HTTP/1, TLS, WebSocket and deliberately misbehaving servers (one per §11.6 limit); SIGKILL injection for durability.
- **Performance targets on fixed reference hardware** (an Apple M-series laptop and a 4-vCPU Linux VM): added latency p99 < 2 ms; ≥ 5k req/s pass-through; RSS < 500 MB regardless of history size; ingest ≥ 2k exchanges/s at a stated body-size mix; 6–8 named history queries with p50/p95 targets warm and cold; UI 60 fps scrolling and filter applied < 300 ms at 1M rows. Macro benchmark (`oha`) with a stored baseline; criterion micro-benchmarks.
- **UI:** Playwright for core flows plus the XSS corpus.
- **Contributor DX:** `just dev`, `just test`, `just fuzz-smoke`; devcontainer; Rust contributors never touch the UI and vice versa; ADRs in `docs/adr/`.

## 14. Roadmap

**Budget rule:** about 12 h/week. If a milestone overruns by 50%, cut features and keep the date.

- **v0.1, core loop (~12 weeks, ~150 h).** Serves bug bounty hunters and students: a free proxy with an exact repeater that never loses work.
  - `crosswire-wire` (HTTP/1, sans-IO) and `crosswire-ca`
  - HTTP/1.1-only MITM proxy; WS/101 tunnelled; TLS passthrough by SNI; raw tunnel fallback
  - store (no FTS); simple field filters; intercept; repeater; match-and-replace; inspector/decoder; tags/notes; copy-as-curl; `crosswire browser`
  - REST API + events; web UI (history, intercept, repeater)
  - CI on Linux and macOS; docs site with a CA-install guide
  - **Definition of done:** golden HTTP/1 suite green; kill -9 durability test green; RSS target met at 1M synthetic exchanges; install-to-first-intercept < 2 min on tier-1 platforms.
- **v0.2.** Serves AppSec engineers and heavier hunters. Upstream HTTP/2 via `h2` adapter (labelled normalized); intruder; full query language + FTS; site map; HAR/Burp XML import; CLI; Windows tier 1; interactsh client for out-of-band checks; read-only MCP; encryption at rest; `crosswire-wire` 0.1 and `crosswire-client` published; r/rust post timed to a working repeater demo; NLnet application.
- **v0.3.** Multi-role replay-and-diff; session rules; WebSocket history and replay; JS plugins (`rquickjs`, typed SDK); passive checks; `crosswire-h2wire` client codec and raw HTTP/2 editing; staged-write timing tests. Two co-maintainers listed in MAINTAINERS.md.
- **v0.4.** Serves small firms: free team mode (central server owns the store; clients proxy locally and stream to it; ULIDs + author; resumable cursors; team-authz RFC first), reporting (Markdown/JSON/SARIF), browser-side HTTP/2, comparer.
- **v0.5+.** HTTP/3, WASM plugins, active scanner framework with community checks, Tauri desktop app.
- **v1.0 criteria:** frozen feature checklist met; passes a fidelity test set that normalising tools fail; RSS ceiling at 1M exchanges; zero data loss under kill -9; 5+ concurrent team users; API v1 and store format frozen. Adoption metrics without telemetry: downloads, Homebrew installs, plugin count, issues from non-maintainers.

## 15. Open source and community
- **Licence:** Apache-2.0 OR MIT (dual). AGPL rejected because it deters embedding of the libraries. Protection against copying is community and speed, not the licence.
- **Process:** CONTRIBUTING, good-first-issues per crate, architecture docs, RFC process for API and plugin changes, code of conduct, release every 4–6 weeks, the maintainer uses the tool in daily work, an archival plan if six months pass without a release.
- **Funding:** GitHub Sponsors and Open Collective; NLnet NGI0 Commons after `wire` 0.1; Sovereign Tech Agency and Alpha-Omega once the libraries have adoption.
- **Launch:** (1) a 90-second video of a request that normalising tools alter being sent exactly; (2) early access for 5–10 bug bounty creators; (3) `crosswire-wire` on r/rust and This Week in Rust; (4) v0.2 on Show HN, r/netsec, r/bugbounty; (5) a conference demo tied to v0.3/v0.4. Also a Burp keymap preset, scope import from a pasted program scope, and a "Coming from Burp" guide.

## 16. Risks
- **Scope creep:** v0.1 frozen to §14; budget rule.
- **HTTP/2 complexity:** data model fixed now (§6); code staged over v0.2–v0.4.
- **"80% then abandoned":** release cadence, definition-of-done per milestone, co-maintainers by v0.3, archival plan.
- **Caido or OpenTRAP close the gap:** our protection is openness, free team mode and the libraries; speed matters.
- **Bus factor:** modular crates, docs, co-maintainers.
- **Tool security:** §11; triage burnout mitigated by the SECURITY.md process.
- **Name:** check GitHub org, domain, USPTO/EUIPO; adopt a short trademark policy; register no placeholder crates.
- **Signing costs and grant dependence:** Sigstore is free; funding never gates the roadmap.
- **Misuse:** the tool targets authorized testing; docs state this; active tools enforce scope.

## 17. Out of scope for v0.1
Intruder, full query language and FTS, CLI, HTTP/2, HTTP/3, WebSocket editing, plugins, MCP, team mode, site map, imports, scanner, AI features, desktop wrapper.
