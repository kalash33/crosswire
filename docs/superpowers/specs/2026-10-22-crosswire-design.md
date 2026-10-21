# Crosswire: design spec (v3)

- **Date:** 2026-10-22
- **Status:** revision 3, after judge panel round 2 (average 6.1/10; the product thesis held across all lenses; the remaining findings were fifteen cross-cutting decisions that every crate depends on). This revision writes those decisions down (M1–M15) and most SHOULD items.
- **Working name:** Crosswire. "CrossWire" (a Bible-software project) exists; the name needs a GitHub org, domain and trademark check before any publish (§16). No placeholder crates are registered.
- **Market facts** carry a "checked 2026-10-22" date and are listed in §2 with sources.

## 1. What it is

Crosswire is an open-source web application security testing platform for authorized testing: an intercepting proxy, request history and a repeater now; intruder, session rules, multi-role testing, team mode, plugins and a scanner later.

**What v0.1 does (shipped):** a free, fast proxy and repeater that runs as a sidecar beside Burp or Caido. It never corrupts a project, bounds its memory at 1M+ requests, and sends what you wrote with no hidden changes.

**Roadmap pitch (dated in §14):** intruder and HTTP/2 (v0.2), bring-your-own AI via MCP with nothing leaving your machine (v0.2b), plugins (v0.3), free self-hosted team mode (v0.4).

**Contributor and library pitch:** written in Rust on a family of small sans-IO crates (§5). The headline library is `crosswire-h1`: no sans-IO HTTP/1 library exists today that parses tolerantly with diagnostics, exposes byte spans, and writes byte-exact.

**Sustainability:** pure community open source. No paid tiers, no closed parts. Funding never gates the roadmap.

**Builder:** one part-time maintainer, 12 h/week, pairing with an AI assistant. From v0.2 the budget splits 8 h build / 4 h community.

## 2. Why it exists

Checked 2026-10-22:
- **Burp Suite Pro:** $499/user/year plus expiring AI credits (portswigger.net/burp/pro); about 88k customers (portswigger.net/burp/ecosystem). Forum and HN complaints: memory use, project files that corrupt or grow to multi-GB, no native collaboration, dated UI.
- **Caido:** closed-source, Rust, $200/year; about 36k new users in 2025 (caido.io 2025 review); free tier 2 projects / 3 plugins; team mode paid; outbound HTTP/2 incomplete (github.com/caido/caido/issues/439, open since 2023); scanner is a community plugin; 29 plugins added in 2025.
- **ZAP:** Java; Checkmarx hired its three leads in 2024. **Hetty:** 12.5k stars, last release 2022. **mitmproxy:** developer tool. **OpenTRAP:** announced 2026-10-19, Java on ZAP, unevaluated.

### 2.1 Prior art for the wire layer

| Project | What it does | Precise gap |
|---|---|---|
| `httparse` | Zero-copy HTTP/1 head parser with lenient `ParserConfig` toggles | No diagnostics, no body framing, no writer, no stable spans |
| `hyper` | Production client/server | Normalises framing and headers; refuses invalid messages by design |
| `rama` / `rama-http-core` | Proxy framework with a hyper fork aimed at compliant emulation; the closest project to ours | Not sans-IO; rejects malformed input; no spans; no diagnostics; httparse-based |
| `h2` | HTTP/2 | Enforces the spec; cannot emit non-compliant frames or header blocks |
| `proxelar` (`proxyapi`) | Rust MITM workbench; stack published | hyper-based, so it inherits normalisation |
| `hudsucker`, `http-mitm-proxy` | MITM libraries | hyper-based |
| `tungstenite` | WebSocket | Compliant only |

**`crosswire-h1` charter:** parse tolerantly with diagnostics; write byte-exact; expose spans for every element; sans-IO. **Non-goal:** a general-purpose HTTP client. A competitor adopting the crates is an accepted outcome and counts as library adoption; the application and community are the moat.

### 2.2 Competitive matrix (honest; cells marked *untested* are filled only after §2.3 runs)

| | Burp Pro | Burp Community | Caido paid | Caido free | ZAP | mitmproxy | **Crosswire v0.1.0** |
|---|---|---|---|---|---|---|---|
| Open source | no | no | no | no | yes | yes | **yes** |
| Price | $499/yr | free | $200/yr | free | free | free | **free** |
| Exact-bytes HTTP/1 repeater | untested | untested | untested | untested | untested | untested | **yes, by §2.3 corpus** |
| HTTP/2 | yes | yes | beta | beta | yes | yes | no (v0.2a) |
| Scanner | best | no | plugin | plugin | yes | no | no |
| Extensions | ~300 | yes | ~49 | 3 max | many | addons | no (v0.3) |
| Team mode | no | no | paid | no | no | no | no (v0.4) |
| Bring-your-own AI | credits | no | yes (paid) | no | no | no | no (v0.2b, MCP) |
| Bounded memory at 1M requests | unverified | unverified | unverified | — | unverified | unverified | **target, measured §13** |

Where Crosswire loses for a long time: scanner depth and the extension ecosystem.

### 2.3 Fidelity corpus (a v0.1 deliverable)

An in-repo corpus of HTTP/1 message shapes that normalising tools alter: duplicate or conflicting framing headers, obsolete line folding, bare LF, non-canonical header casing and order, whitespace before the colon, trailing-space names, absolute-form targets, chunk extensions, trailers, non-UTF-8 body bytes, leading whitespace before the request line, pipelining, 0-length and >1 MiB bodies. Seeded from the llhttp corpus (MIT) plus at least 200 of our own captures covering every `DiagnosticCode`, with a contributor README. The same corpus is the golden suite (§13), the "altered by hyper/h2" assertion, the launch video script, the v1.0 fidelity test set, and a published docs page with one row per tool, filled by actually running each tool. Minimum size is a DoD number (§14).

## 3. Users, and the release that serves each

1. **Bug bounty hunters and creators** (v0.1): a sidecar beside Burp; exact repeater, no corruption, bounded memory, free.
2. **Students, CTF players, price-sensitive regions** (v0.2a): intruder, Windows tier 1, HTTP/2.
3. **AppSec engineers** (v0.2b): replay a captured exchange byte-exact from CI via the CLI; read-only MCP.
4. **Small pentest firms, 3–30 testers** (v0.4): free team mode, reporting.
5. **Large consultancies** (after v1.0).

## 4. Product principles

1. **Fidelity means no hidden changes.** The recorded bytes are defined in §6.0; conveniences are visible chips that can be turned off and show a diff of what they change.
2. **Never corrupts a project. Loss window ≤ 250 ms, every gap marked** (§9).
3. **Bounded resources.** Memory does not grow with history; every hostile-input dimension has a limit (§11.6).
4. **API-first.** UI, CLI, MCP and plugins are clients of one versioned API.
5. **Free means everything.**
6. **Safe by default** (§11).
7. **Library quality.** Published crates are sans-IO where possible, `#![forbid(unsafe_code)]`, `#![deny(missing_docs)]`, fuzzed, semver-checked; published crates depend only on published crates (enforced with `cargo package` in CI).
8. **No phoning home.** No telemetry; no outbound calls while idle (tested in CI). Update checks and crash reports are opt-in.

### 4.6 Scope: three separate meanings
- **Proxy recording:** everything is recorded by default (matches Burp and Caido; the disk quota bounds it). "Do not record out-of-scope" is an explicit opt-in. Out-of-scope traffic is never intercepted. Blocking it is opt-in.
- **History view:** in-scope only by default, one-click toggle. A "Hosts seen" panel (host, count, one-click add to scope or passthrough) ships in 0.1.x.
- **Active tools (repeater, intruder, CLI send):** sends are allowed by default. When the target is out of scope, or scope is empty, the tab shows a persistent amber badge; the first send to that host per tab asks once (Send anyway / Add to scope / Cancel) and remembers the answer for the tab. Every out-of-scope send appends one audit-log row (§9.4). The project setting **enforce scope strictly** (default off; forced on in team mode and for MCP) restores never-send. **Empty scope never prompts**; the history toolbar offers "Set scope from this host".
- **MCP:** in-scope only, no override (§11.8).
- Scope is evaluated on **both** the resolved connection target plus SNI **and** the Host / `:authority` header. In strict mode the send is refused if either is out of scope. The §13 test matrix covers in-scope Host with out-of-scope target and the reverse, for repeater, intruder, CLI and MCP.

### 4.7 Scope model
Ordered include rules, then exclude rules. A rule = host pattern (exact, `*.` suffix, or regex) + optional port set + optional scheme + optional path prefix or regex. IP literals match only when listed. Precedence: first matching exclude wins over includes; no include match → out of scope. CONNECT-only and passthrough exchanges are scoped by SNI. A URL × rule → in/out test table lives in `crosswire-core`. Scope import from pasted program lists (one host per line; copy-paste from the major platforms' scope tables) is a parser over the same grammar and is in v0.1.

## 5. Architecture

```
clients:  web UI (Svelte/TS)    CLI (v0.2b)    MCP (v0.2b)    plugins (v0.3)
                      │   REST /api/v1 + WS events (OpenAPI → generated TS/Py clients)
                      ▼
┌──────────────── crosswire-server (binary) ─────────────────────┐
│  API listener (loopback)              Proxy listener           │
│            │                                │                  │
│     crosswire-api ──► crosswire-tools ──► crosswire-proxy      │
│                        (repeater, intruder)  (MITM, Handler)   │
│                              │   ▲              │              │
│                              ▼   │              ▼              │
│                      crosswire-rules (M&R, session rules)      │
│                              │                                 │
│                              ▼                                 │
│     crosswire-store ◄── writer thread ◄── bounded queue        │
│     (SQLite WAL + packs + journal + audit)                     │
│              ▲                                                 │
│     crosswire-query (parse/AST/eval; SQL lowering in store)    │
│                                                                │
│  crosswire-client  Transport / Session / Stream, TlsBackend    │
│      ├─ crosswire-h1    sans-IO HTTP/1, exact      (v0.1)      │
│      ├─ crosswire-h2    sans-IO HTTP/2 + HPACK     (v0.3)      │
│      └─ crosswire-ws    sans-IO WebSocket          (v0.3)      │
│  crosswire-core (Span, Diagnostic, Fidelity, Scope)            │
│  crosswire-ca (CA + leaf issuance)   crosswire-model (Message) │
└────────────────────────────────────────────────────────────────┘
```

Assumed rate for the hour column: **60 reviewed, tested, explainable lines per hour** including tests and docs (AI-paired, maintainer must be able to explain every line per §11.7). The budget is falsifiable against `docs/log.md`.

| Crate | Depends on | In workspace from | Published at | Responsibility | LoC | Hours |
|---|---|---|---|---|---|---|
| `crosswire-core` | `bytes` | v0.1 | v0.2a (first 4 weeks) | `Span`, `Diagnostic`, `DiagnosticCode`, `Profile`, `Fidelity`, `Severity`, scope grammar + evaluator. Aim for 1.0 early. | 1.5k | 25 |
| `crosswire-h1` | core | v0.1 | v0.2a (first 4 weeks) | Streaming tolerant HTTP/1 parser, exact writer, body codec, edit engine, diagnostics (§5.1). | 6k | 100 |
| `crosswire-h2` | core | v0.3 | v0.3 | Frame codec, HPACK with encoding hints, non-compliant emission on request, sans-IO connection state machine (streams, flow control, SETTINGS, CONTINUATION). | 6k | 100 |
| `crosswire-ws` | core | v0.3 | v0.3 | Sans-IO WebSocket frame codec. | 1k | 17 |
| `crosswire-client` | core, h1, (h2) | v0.1 | v0.3 | tokio I/O: `Transport`, `Session`, `Stream`, `WritePlan`, `TlsBackend` (rustls default), HTTP/SOCKS5 chaining; `pool` feature, never implicit (§5.2). | 3k | 50 |
| `crosswire-ca` | `rcgen` | v0.1 | v0.2a | Thin: CA creation, SNI leaf issuance, cache, DER/PEM export; `keychain` feature. Browser/OS trust installation lives in the server/CLI. | 1k | 17 |
| `crosswire-model` | core | v0.1 | 1.0 | `Message`, `Exchange`, events, redaction policy. | 1k | 17 |
| `crosswire-rules` | core, h1 | v0.1 | 1.0 | Match-and-replace, `${var}` variables, session rules (v0.3). | 1k | 17 |
| `crosswire-query` | core | v0.2b | 1.0 | Filter language parser, AST, type-checker, `FieldRegistry`, in-memory evaluator. | 2k | 33 |
| `crosswire-store` | model | v0.1 | no | SQLite + packs + journal + audit; SQL lowering of queries. | 3.5k | 58 |
| `crosswire-proxy` | client, rules, model | v0.1 | after a stable `Handler` | MITM engine; v0.2a `h2` adapter lives here. | 3k | 50 |
| `crosswire-tools` | client, rules, model | v0.1 | no | Repeater (v0.1), intruder (v0.2a), decoder, comparer (v0.2a). | 2k | 33 |
| `crosswire-api` | all above | v0.1 | no | REST + WS handlers, OpenAPI via `utoipa`, credential model (§11.2). | 2.5k | 42 |
| `crosswire-server` | all | v0.1 | binary | Composition, listeners, embedded UI, browser launcher, CA trust install. | 1.5k | 25 |
| `crosswire-cli` | api client | v0.2b | binary | Headless commands. | 1k | 17 |
| `ui/` | — | v0.1 | — | Svelte + TS: history, intercept, repeater, inspector, settings; virtualised tables; CodeMirror 6; Burp-compatible keymap. | 8k | 133 |
| harnesses, corpus, CI, docs, release pipeline | — | v0.1 | — | §13 | 3k | 50 |

**v0.1.0 total (core + h1 + client + ca + model + rules + store + proxy + tools-repeater + api + server + ui + harnesses):** about 18k Rust + 8k TS → **~300 h** at 60 LoC/h, ≈ 24 weeks at 12 h/week.

**Contribution status:** open for contribution from alpha.1: ui, tools, docs, ca, corpus. Design-locked until 0.2: core, h1, store, api.

**Why REST + OpenAPI (not GraphQL):** generated typed clients, a clean MCP mapping, trivial curl/CI use, one familiar style.

### 5.1 `crosswire-h1` API sketch

```rust
// crosswire-core
pub struct Span { pub start: u32, pub end: u32 }        // exclusive; bounded by Limits.head_size
#[non_exhaustive] pub enum DiagnosticCode { DuplicateContentLength, ConflictingFraming, ObsFold, BareLf, WhitespaceBeforeColon, /* … */ }
pub struct Diagnostic { code: DiagnosticCode, span: Span, severity: Severity }
#[non_exhaustive] pub enum Profile { Strict, Lenient /* default */ }
#[non_exhaustive] pub enum Fidelity { Raw, DecodedOrdered, Normalized, Opaque, Generated }

// crosswire-h1  (depends on bytes only; `http` and `serde` are optional features)
pub struct RawMessage { /* private */ }                   // immutable; accessors only
impl RawMessage { pub fn bytes(&self) -> &Bytes; pub fn head(&self) -> &HeadView; pub fn diagnostics(&self) -> &[Diagnostic]; }
#[non_exhaustive] pub enum Kind { Request, Response }
pub enum StartLine { Request { method: Span, target: Span, version: Span }, Response { version: Span, status: Span, reason: Span } }
pub struct HeadView { start: StartLine, headers: Vec<HeaderSpan>, framing: BodyFraming }
pub struct HeaderSpan { name: Span, value: Span, line: Span }
#[non_exhaustive] pub enum BodyFraming { None, ContentLength(u64), Chunked, UntilClose, Ambiguous(Vec<BodyFraming>) }

pub struct Parser { /* streaming state machine */ }
impl Parser {
    pub fn request(profile: Profile) -> Self;
    pub fn response(profile: Profile, ctx: ResponseContext /* {method, is_connect} so HEAD/1xx/204/304/CONNECT frame correctly */) -> Self;
    pub fn feed(&mut self, input: Bytes) -> Result<Progress, ParseError>;   // spans reference caller-owned Bytes
    pub fn feed_eof(&mut self) -> Result<Progress, ParseError>;
}
#[non_exhaustive] pub enum Progress { NeedMore, Head(HeadView), BodyData { raw: Span, data: Span }, ChunkExt(Span), Trailers(Vec<HeaderSpan>), Done }
pub struct BodyDecoder; pub struct BodyEncoder; pub struct ChunkSpan { size_line: Span, ext: Option<Span>, data: Span, trailer: Option<Span> }

pub fn write_exact(msg: &RawMessage, out: &mut BytesMut);   // byte-identical, always
pub struct Edit { span: Span, replacement: Bytes }
pub struct Change { op: ChangeOp, span: Span, before: Bytes, after: Bytes, reason: &'static str }
pub fn apply_edits(msg: &RawMessage, edits: &[Edit]) -> (RawMessage, Vec<Change>);  // the ONE edit entry point:
// used by match-and-replace, intruder positions, the Content-Length chip, hop-by-hop stripping. Every edit yields new bytes, re-parsed.
#[cfg(feature = "http")] impl TryFrom<&RawMessage> for http::Request<()> { /* documented lossy */ }
#[cfg(feature = "http")] impl TryFrom<&RawMessage> for http::Response<()> { /* documented lossy */ }
```

**Feature matrix:** h1 `std` (default), `http`, `serde`; client `rustls` (default), `native-tls`, `pool`, `socks5`; ca `keychain`. CI builds each publishable crate with `--no-default-features` and with each feature alone. `no_std + alloc` and wasm32 builds of core and h1 are CI targets (unlocks v0.5 WASM plugins).

### 5.2 `crosswire-client` types
- `Transport`: bytes over TCP, TLS via `TlsBackend`, HTTP CONNECT / SOCKS5 chaining (optional auth, per-host include/exclude; "chain with Burp" is a documented flow).
- `Session`: one connection's protocol state: h1 serial, or h2 multiplexed (state machine lives in `crosswire-h2`; client only pumps I/O).
- `Stream`: one exchange; owns `write_plan(WritePlan)` where `WritePlan` = ordered `Segment { bytes_or_frame, hold: Option<HoldPoint> }`. Timing-sensitive sends later need no public break.
- `TlsBackend` (object-safe, runtime-agnostic): connect with `TlsConnectConfig { sni: Option<..>, alpn, verify: VerifyMode, client_cert }`; expose negotiated version, cipher, ALPN, chain and validation result; server-side resolve-cert-by-SNI. ClientHello shaping is an explicit rustls non-goal; the trait is the plug-in point for a second backend (v0.3 list). The rustls fingerprint and the fact that bot-management challenges are expected are documented.
- Pooling only behind the `pool` feature and an explicit `reuse` choice per send.

## 6. Fidelity contract

### 6.0 What is recorded
- **Canonical request** = the bytes written to upstream. **Canonical response** = the bytes read from upstream. "Raw" means exactly that.
- Browser-side bytes are recoverable from the stored typed diff list: hop-by-hop strip, `Proxy-*` strip, absolute-form → origin-form rewrite, each match-and-replace hit.
- "Send to repeater" loads upstream-side bytes.
- **Connection model:** one browser connection ↔ one upstream connection; keep-alive preserved; requests paired in order; pipelining tolerated serially; `Connection: close` ends recording for that connection.
- **Framing resolution:** on `Ambiguous`, the relay delimits by RFC 9112 precedence, emits the diagnostic, forwards bytes unchanged, records the framing used. An undelimitable stream falls to a raw tunnel for the rest of that connection plus an error event. `Expect: 100-continue` is relayed as-is. One golden capture per case.

### 6.1 `Message` schema (frozen in v0.1; shared by API and store; lives in `crosswire-model`)
```
Message {
  kind: http1-raw | http2-frames | normalized | ws-frames
  fidelity: raw | decoded-ordered | normalized | opaque | generated
  head:  http1-raw   → { bytes, spans }
         http2-frames→ { frames: [{seq /*per-connection monotonic*/, stream_id, type, flags, payload}], headers: [{name, value, hint: EncodingHint}], table_snapshot_ref }
         normalized  → { map }           // translated header list; frames absent, hints null
  body:  inline { bytes } | blob { hash, len, truncated_at, capture_dropped }
  diffs: [{ op, span, before, after, reason }]
  connection: { target_addr, port, sni, alpn, tls_version, upstream_chain_result, conn_id, reused }
}
Exchange { id: ULID, request: Message, response: Message?, timing, scope_state, tags, notes, namespace: history|repeater|intruder, state: received|committed|error }
```
- HTTP/2 frames carry a per-connection sequence plus stream id, because HPACK state is per connection; per-stream views are derived. Golden tests round-trip **frame for frame per connection**.
- **`normalized` path (v0.2a):** HTTP/2 via the `h2` adapter stores the frame schema with frames absent and hints null plus the translated header list; the h1→h2 translation rules are listed in `docs/format.md`; the repeater shows a "normalized: HTTP/2 via h2" banner, disables raw chips, and offers "force HTTP/1.1 upstream" per target.
- **Labels in the UI:** shown in the table only when not `raw`; always in the detail pane. Tunnelled WebSocket and SNI-passthrough connections render as `opaque` rows with byte counters and duration. Proxy-authored 502/504 pages are `generated`.

### 6.2 Protocol support matrix

| Release | Browser ↔ proxy | Proxy ↔ upstream | Fidelity |
|---|---|---|---|
| v0.1.0 | HTTP/1.1 only (ALPN offers http/1.1); WS/101 tunnelled; SNI passthrough list; auto-passthrough on repeated handshake rejection (§7.1) | HTTP/1.1 | raw / opaque / generated |
| v0.2a | HTTP/1.1 | + HTTP/2 via `h2` adapter | + normalized |
| v0.3 | HTTP/1.1 | HTTP/2 via `crosswire-h2` (client role) | + decoded-ordered |
| v0.4 | + HTTP/2 (server role, same codec) | HTTP/1.1 + HTTP/2 | decoded-ordered |
| v0.5+ | + HTTP/3 | + HTTP/3 (`quinn`) | decided by ADR |

## 7. Behaviour

### 7.0 History view (v0.1)
Default columns: #, time, method, host, path, status, length, MIME, TLS, response ms, fidelity (when not raw), tags, notes. Sortable, multi-select, context menu (send to repeater, copy URL / curl / raw, add host to scope or passthrough, tag). Uncommitted rows are shown distinctly (§9). Search box in 0.1.x: case-insensitive literal and regex "contains" over head and body as a streaming scan with progress and a result cap; FTS5 is the v0.2a upgrade of the same box.

### 7.1 Proxying
1. Browser sends CONNECT.
2. SNI checked against the passthrough list and the auto-passthrough list; listed hosts are tunnelled as `opaque`.
3. TLS is terminated with a cached leaf cert. **Rejected handshakes:** on N consecutive client-side rejections for a host within a window, close the connection, record an error event, toast "Browser rejected the Crosswire certificate for X — is the CA trusted?", and add the host to a user-visible "Auto-passthrough (handshake rejected)" list with clear/retry; the *next* CONNECT is tunnelled. Auto-passthrough defaults **off for in-scope hosts** (they surface the error loudly). Playwright test: "install CA after first failure".
4. `crosswire-h1` parses incrementally; scope state is computed; intercept rules (§7.3) and match-and-replace (§7.4) apply.
5. The request is forwarded through `crosswire-client` under the upstream TLS policy (§11.9); the response is relayed.
6. The exchange is queued to the store: a `received` event is emitted on enqueue and `committed` after the batch commits (§9).

**Body model:** head = owned bytes + spans. Body inline up to `Limits.inline_memory` (1 MiB), otherwise streamed: relayed while copied to the blob writer with a bounded window. The relay never waits on capture; if the capture window is full the body is marked `capture_dropped` while length and hash still come from the relay path. A capture cap keeps the first N MB plus full hash and length; truncation is shown.

**WebSocket in v0.1:** after a 101, bytes are relayed unparsed as an `opaque` exchange with counters; the upgrade exchange is logged. Frame history, editing and replay: v0.3.

**Handler trait (for the proxy crate's eventual publication):** `on_request_head`, `on_request_body_chunk`, `on_response_head`, `on_response_body_chunk`, each returning `Forward | Hold | Modify(Vec<Edit>) | Drop`.

### 7.2 Repeater (v0.1)
- A new tab starts in **assisted** mode: Content-Length update on, line-ending normalisation on, both shown as chips. **Raw** is a per-tab toggle that shows a diff of what assisted mode would have changed. Follow redirects: off / in-scope / always.
- Connection settings: target host, port, TLS and SNI separate from the Host header; fresh vs reused connection.
- Paste-raw-request (CRLF and encoding preserved) and copy-as-raw are first-class flows with a Playwright test. Response TTFB and total time in the result header.
- Drafts and connection settings autosave to the project (debounced). Per-tab send history (bounded), tab rename/reorder/group: 0.1.x.
- Results live in the `repeater` namespace linked to the source exchange, purgeable per tab.

### 7.3 Intercept (v0.1)
Off at project open; toggle in the status bar. Default rules skip image/font/css/js MIME types and out-of-scope hosts. Rules: simple field filters in v0.1, the full query language in v0.2b. Actions: forward, edit-and-forward, drop, forward-all, send-to-repeater, "intercept response to this request". Heads are held with bodies up to `Limits.intercept_hold`; larger bodies stream through marked "not editable". Queue overflow action (forward newest / oldest) and hold-timeout action (requests: forward / 504; responses: forward / drop) are separate settings. HTTP/2 per-stream holds: v0.4.

### 7.4 Match-and-replace (v0.1, `crosswire-rules`)
Ordered rules on request/response head or body, literal or regex (the linear-time `regex` crate with a compiled-size limit), applied through `apply_edits` and recorded in the exchange diff. `${var}` placeholders are reserved from v0.1 so v0.3 session rules (extract by regex/JSONPath → variable → inject; login macro on 401/302) need no rewrite.

### 7.5 Intruder (v0.2a)
Positions are byte ranges in the raw request; length fields adjust only when the Content-Length chip is on. Attack types: sniper, battering ram, pitchfork, cluster bomb. Payload sets, grep-match and extract columns. Bounded concurrency, rate limit, pause/stop. Results in a per-attack namespace with purge.

### 7.6 Small daily tools (v0.1)
Inspector/decoder pane (URL, base64, hex, JWT view, HTML entities); tags and notes; copy-as-curl (every argument single-quoted via a documented routine tested against a hostile-header corpus for sh and PowerShell); `crosswire browser`; scope import (§4.7); Burp-compatible default keymap, remappable, covering send-to-repeater, send, forward/drop, toggle intercept, decode selection, tab switching; Playwright test that the core loop runs mouse-free. Comparer (word/byte diff of two exchanges): v0.2a.

### 7.7 Project lifecycle
Launch opens a temporary project (auto-delete or auto-keep per setting); "Save as" promotes it; recent list; switch project without restarting listeners; delete with confirmation. Repeater tabs, drafts and scope travel with the project.

## 8. API and events
- REST under `/api/v1`, OpenAPI-described; N-1 deprecation; stable error codes. Keyset pagination only.
- WebSocket event stream with monotonic sequence ids; bounded per-subscriber buffer; `lagged` + resync from cursor; events filtered per subscriber (team-ready). Event kinds include `received`, `committed`, `error`, `limit_exceeded`, `gap`.

## 9. Storage

### 9.1 Layout
A project is a directory (0700): `project.db` (SQLite WAL, `synchronous=NORMAL`, `application_id`, `user_version`), `packs/`, `journal/`, `audit.jsonl`, `project.lock` (flock on open). Format versioned in `docs/format.md` from v0.1; snapshot before migration; an older binary refuses a newer project. Foreign projects open in SQLite defensive mode with `trusted_schema=OFF`.

### 9.2 Bodies and packs
Bodies < 16 KiB inline in SQLite; larger in append-only packs. Pack record = `{magic, len, sha256, zstd frame}`; one reserved header field for a future encryption ADR. Blob table `{hash, pack_id, offset, len, refcount}`; deduplicated by hash; hashes verified on read; zstd frame-size and ratio caps on read. A pack segment is fsynced before the metadata row that references it commits. Startup sweep validates from the last checkpointed offset, truncates a torn tail, and deletes rows whose blob is missing. Purge decrements refcounts; compaction rewrites packs below a live ratio.

### 9.3 Write path and durability
- The proxy never performs file I/O. Exchanges go to a bounded queue (by bytes, 64 MiB, and by count); one OS writer thread drains it and commits batches every 250 ms or 1,000 exchanges, whichever first. Queue full → `try_send` to a spill thread that appends to `journal/`; if that fails too (disk full) the exchange is dropped with a `gap` marker and the proxy **keeps relaying and stops recording**, with a persistent alert. Traffic is never blocked. Journal is replayed on startup.
- **Guarantee (a), process crash (kill -9):** every exchange whose `committed` event was broadcast is recoverable; at most 250 ms of exchanges plus the queue contents may be lost, and the gap is marked. **Guarantee (b), power loss / OS crash:** committed-and-checkpointed data survives; WAL checkpoint interval is 30 s or 64 MiB.
- Reads: a fixed pool of K rusqlite connections on blocking threads, bounded `cache_size` and `mmap_size`, statement timeouts, so RSS can be computed. A global in-flight body byte budget is shared by proxy, intercept and repeater.
- Retention: per-project size/age quota; audited purge clears all namespaces, FTS rows, `journal/`, WAL (checkpoint + truncate) and unreferenced blobs, with `secure_delete=ON`.

### 9.4 Audit log
Append-only `audit.jsonl` (ULID, actor = ui/cli/mcp/plugin id, action, target, result), 0600, in the project directory; history purge never truncates it (separate audited `audit purge`); team mode ships it to the server. Audited actions: scope changes, out-of-scope sends, exports, purge, token create/rotate/revoke, CA rotate, plugin install or capability change, MCP sends, upstream-cert override.

### 9.5 Search (v0.2a)
FTS5 external content, trigram tokenizer, built asynchronously and capped per body; redacted fields excluded (§11.4). `~` = case-insensitive substring; `matches` = regex, prefiltered by trigrams only when a ≥3-byte literal can be extracted, otherwise a bounded scan with a row cap; `matches` on body fields without a literal is rejected with a typed error.

## 10. Filter language
- **v0.1:** simple field filters (method, host, path, status, MIME, size, tags, in-scope) in UI and API.
- **v0.2b:** full language, e.g. `req.method == "POST" and resp.status >= 500 and req.host ~ "api."`. Parse → AST → type-check against `FieldRegistry`; evaluate in memory (intercept rules, live filters) or lower in `crosswire-store` to parameterised SQL only. Grammar documented; parser fuzzed.

## 11. Security of the tool itself

### 11.0 Threat model

| Asset | Adversary | Controls |
|---|---|---|
| API session / tokens (full control) | hostile target content in the UI; hostile web page; local user | §11.1, §11.2, route-auth test |
| CA private key | local user; malware; team server | §11.3; never in API, export or sync |
| Captured credentials and PII | local user; exports; plugins; teammates | 0700/0600; redaction policy §11.4; plugin gating §11.5 |
| The user's non-target traffic (banking, SSO in the same profile) | the network; a hostile upstream | upstream TLS verification §11.9; SNI passthrough |
| The proxy process | hostile servers and clients | `Limits` §11.6; `forbid(unsafe_code)` in sans-IO crates |
| A project directory or import file | hostile file | defensive mode, zstd limits, bounded and fuzzed sweep/migrations/importers, hashes verified on read |
| Team server (v0.4) | a teammate; the network | roles, hashed tokens, filtered events, audit log, egress denylist, CA never on the server |
| Users and the build | supply chain | §11.7 |
| Out-of-band callback service (v0.2b) | the service operator | opt-in per project; server configurable |

Trust boundaries: browser ↔ proxy listener; UI/CLI/MCP ↔ API listener; server ↔ upstream; server ↔ plugin worker (v0.3); client ↔ team server (v0.4); server ↔ OOB service (v0.2b). Every feature updates this table before it merges.

### 11.1 Untrusted content rendering
Captured bytes render as text only (CodeMirror or text nodes). A CI lint bans `{@html}`, `innerHTML`, `insertAdjacentHTML`, `srcdoc`, `javascript:` hrefs and `<svelte:element this>` in the UI. HTML/image previews are served from a **different hostname** than the API (API on `localhost`, previews on `127.0.0.1`) so the session cookie is never sent, in a sandboxed iframe without `allow-same-origin`, with `Content-Security-Policy: sandbox; default-src 'none'; img-src data: blob:` and `Referrer-Policy: no-referrer`. The UI is served with `default-src 'self'`, a per-response nonce wired to CodeMirror, `X-Content-Type-Options: nosniff`, `frame-ancestors 'none'`. The CLI strips terminal control sequences from captured content. XML imports disable DTDs; importers are fuzzed; export filenames are sanitised. Playwright tests load an XSS corpus into history, repeater and intercept and assert nothing executes and that a preview with `<img src=…>` makes zero outbound requests.

### 11.2 Listeners and credentials
- **Listeners:** API and proxy are separate sockets. The API binds to loopback only (`--api-bind` elsewhere prints a warning; Docker users publish `-p 127.0.0.1:PORT:PORT`). The proxy may bind non-loopback only with a flag and a warning, and then **proxy authentication is mandatory**. The proxy always refuses to forward to loopback, link-local and its own listener addresses, evaluated **after DNS resolution** regardless of spelling, unless a per-destination allowlist entry exists; the §13 test iterates loopback spellings. A Unix-domain-socket option for the API is considered for Linux/macOS.
- **Credentials:** bootstrap token ≥128 bits, single-use, 60 s expiry, accepted only by `POST /api/v1/session`; the UI receives it once in the URL fragment and clears it with `history.replaceState`. Session cookie: HttpOnly, SameSite=Strict, `Path=/api`, name `cw_sid_<port>`, idle expiry. API tokens are scoped (`read`, `send`, `admin`), project-bound, hashed server-side, shown once, individually revocable; MCP and plugin tokens are ordinary scoped tokens enforced server-side; constant-time comparison; `crosswire token rotate` lists what it invalidates.
- **Request checks:** reject any Host not `localhost` / `127.0.0.1` / `[::1]` / the configured bind name with 421; mutating routes require `Sec-Fetch-Site: same-origin` or an allowlisted Origin, or a bearer token. The only unauthenticated routes are static assets, session bootstrap and `GET` of the CA public certificate; the route-auth test asserts everything else.

### 11.3 CA handling
Key in the OS keychain by default (`keychain` feature; DPAPI on Windows with an ACL to the current user's SID), 0600 file fallback, stored in the user config dir and never under a project. CA: `basicConstraints CA:TRUE`, `keyUsage keyCertSign`, unique subject per install, one-year validity. Leaves: SAN, EKU serverAuth, ≤398-day validity, notBefore −1 day; one P-256 leaf keypair per install; LRU cache with a disk cap and a mint-rate limit. Trusted only in the `crosswire browser` profile by default; OS-wide trust is opt-in. `ca rotate`, `ca uninstall`, DER/PEM export with Android (7+ user-CA limitation documented) and iOS instructions. A test asserts the private key never appears in any API response, export or sync.

### 11.4 Sensitive data and redaction
One user-extensible redaction policy in `crosswire-model`, used by exports, FTS, plugin reads, MCP reads, copy-as-curl and logs: a default header set (Authorization, Proxy-Authorization, Cookie, Set-Cookie, X-API-Key, X-Auth-Token, X-Amz-Security-Token, …) plus value patterns (JWT, `Bearer …`, `access_token=`, `api_key=`). Exports print a redaction summary; a golden test asserts an exported HAR of a seeded project contains none of the seeded secrets. Directories 0700, files 0600. Logs and crash reports never contain captured bytes. Encryption at rest is not designed yet: ADR required, v0.3+ (§17).

### 11.5 Plugins (v0.3)
The worker process is the sandbox boundary and holds no secrets, only a scoped plugin token and declared capabilities. The SDK exposes no filesystem, process or socket primitives; all I/O goes through the API with server-side capability checks. Memory and CPU limits; per-call timeout on in-path hooks with a documented fail-open/closed setting; seccomp/Landlock on Linux, best effort elsewhere. Network access limited to declared hosts in scope. Store reads redacted unless a `secrets` capability is granted. UI plugins run in a sandboxed iframe. Install from a local file or a curated hash-pinned index with the manifest shown; no auto-update; users are re-prompted when capabilities grow.

### 11.6 `Limits` (one table; every entry has an action and an observable)

| Limit | Default | Action when hit |
|---|---|---|
| `inline_memory` | 1 MiB | body streams to blob |
| `inline_sqlite` | 16 KiB | body goes to pack |
| `intercept_hold` | 4 MiB | body not editable, streams through |
| `capture_cap` | 50 MiB | `truncated_at` set; hash and length from relay path |
| store queue | 64 MiB / 10k | spill to journal → drop with `gap` |
| in-flight body budget | 256 MiB | new bodies `capture_dropped` |
| head size / header count / line length | 64 KiB / 256 / 16 KiB | stop reading, retain ≤ limit + tail, close, `limit_exceeded` record; fixture asserts flat RSS |
| decompression size / ratio | 64 MiB / 100:1 | derived view marked truncated |
| per-exchange timeouts (header read, idle, total) | 30 s / 60 s / 300 s | `generated` 504 |
| max connections per listener; pending CONNECTs; per-client-IP (non-loopback) | 4k / 256 / 64 | refuse with a counter |
| SNI | ≤ 253 bytes, valid hostname | reject CONNECT |
| leaf-cert LRU count / disk / mint rate | 4k / 64 MiB / 50 per s | evict; rate-limit |
| DNS timeout / concurrency | 5 s / 64 | `generated` 502 |
| HPACK table / CONTINUATION total / streams (v0.3) | 64 KiB / 1 MiB / 256 | diagnostic, close |

Hostile-server and hostile-client fixtures exist per row (§13).

### 11.7 Supply chain and governance
SECURITY.md with private reporting via GitHub Security Advisories, 72 h acknowledgement, 90-day window. DCO sign-off. Binaries: MSRV = stable-2. Published crates carry `rust-version` with MSRV = stable-6; an MSRV bump is a minor release; crates version independently; 0.x breaking changes are batched and listed via cargo-semver-checks. cargo-deny, cargo-audit, osv-scanner, Dependabot in CI. Actions pinned by SHA. `npm ci --ignore-scripts` with a lockfile; no generated, minified or vendored artefacts committed; the UI is built from source in release CI. SBOM (cargo-cyclonedx) on releases; Sigstore-signed releases with SLSA provenance; cosign-signed Docker image; binstall metadata with signature verification; crates.io trusted publishing; hardware 2FA on the org root. Per-crate changelog via release-plz; one-command release pipeline (release-plz + cargo-dist). AI-assisted PR policy, applied to the maintainer too: tests required, the author must be able to explain the change, wire-layer changes need golden tests, a module-level design doc per crate before merge, an external review requested before `crosswire-h1` 0.1 and recorded as an ADR.

### 11.8 MCP (v0.2b)
stdio transport only. Its own scoped token; read-only by default. `send` requires a per-session capability with a TTL and a human-approval mode (approve each send or a batch of N), default on; in-scope only; no override. Per-call and per-minute caps; size caps and mandatory pagination. Reads redacted under §11.4 unless `secrets` is granted. Never exposes scope override, export, plugin install or token operations. Target-derived content is labelled untrusted data. A §13 test injects an instruction-like body and asserts no send happens without approval.

### 11.9 Upstream TLS policy
Verify upstream chains against OS/webpki roots for every host by default. On failure do not relay: return a `generated` 502 and emit an event. "Accept invalid upstream cert" exists only per host or per scope entry, never globally, and is audit-logged. Record the chain validation result, version and ALPN on every exchange (§6.1). Client certificates for mTLS targets are configurable per host. SNI passthrough is the documented way to keep the browser's own validation for sensitive sites. A §13 fixture presents a bad chain and asserts refusal by default and acceptance only with the per-host override.

## 12. Configuration, platforms, distribution
- **Config:** per-OS paths; precedence flags > env > project > user > defaults; per-project overrides; listeners; first-run token.
- **Platforms:** tier 1 = Linux x86_64/aarch64, macOS aarch64; tier 2 = macOS x86_64, Windows x86_64 (CI nightly, non-blocking until v0.2a, when Windows becomes tier 1 and the DPAPI/ACL tests are required).
- **Distribution:** single static binary with the UI embedded; `cargo binstall` with signature verification; Linux tarball + Sigstore; macOS Homebrew **cask** with a Developer ID-signed and notarized binary once the Apple Developer Program fee (~$99/yr, the first recurring cost, paid from Sponsors) is covered, unsigned tarball before that; Windows unsigned with the SmartScreen caveat until funded; Docker image.
- **Browser launcher (`crosswire browser`):** Chromium = separate `--user-data-dir` + `--ignore-certificate-errors-spki-list=<CA SPKI>` (documented as disabling verification only for chains containing that SPKI); Firefox = CA written into the profile's cert9.db (0.1.x). One persistent profile per project (configurable per user), `reset profile`, pass-through flags and extensions. Chromium-only is enough for v0.1.0.
- **"Install to first intercept < 2 min"** is a scripted test: install via cargo-binstall (no Gatekeeper quarantine), launch, fetch `https://example.com` through the launched browser, assert the exchange appears.
- **Updates:** opt-in update check; `/api/v1` N-1 deprecation; project-format migrations with snapshot.

## 13. Testing and performance
- **Golden fidelity suite** = the §2.3 corpus: HTTP/1 byte-for-byte; HTTP/2 (v0.3) frame-for-frame per connection; one capture per §6.0 framing case.
- **Fuzzing** (`cargo-fuzz`): h1 parser and edit engine, query parser, importers, pack reader; h2 frames/HPACK in v0.3. Smoke in CI, full nightly.
- **Property tests** (`proptest`) for parse/write round-trips and `apply_edits` span arithmetic.
- **Integration harness:** in-process HTTP/1, TLS (including a bad-chain server), WebSocket and deliberately misbehaving servers and clients, one per `Limits` row; SIGKILL injection at random points under load, restart, `integrity_check` ok, every `committed` exchange present, gap marker covers the rest, repeater drafts restored.
- **Security gates in CI (v0.1.0 DoD):** route-auth test; `read`-token-cannot-mutate; XSS corpus; preview zero-egress; idle-egress; CA-key-leakage; loopback-spelling relay refusal; scope matrix (§4.6); redaction golden; bad-chain refusal.
- **Performance** (`bench/README.md`; reference hardware: an Apple M-series laptop and a 4-vCPU Linux VM): added latency p99 < 2 ms per request on established browser and upstream connections, MITM TLS on, recording on, 64 keep-alive connections, 16 KiB body; ≥ 5k req/s pass-through; RSS < 500 MB after loading 1M synthetic exchanges, running all named queries and scrolling (peak RSS); ingest ≥ 2k exchanges/s at a body mix of 60 % < 4 KiB, 30 % 4–64 KiB, 9 % 64 KiB–1 MiB, 1 % > 1 MiB; named history queries (last 200 by ULID; host = X; status ≥ 500 and method = POST; path matches regex; size > 1 MiB; tag = Y; in-scope only; text in path) with p50/p95 warm and cold at 1M rows, from which the §9 indexes are derived; UI 60 fps scrolling and filter applied < 300 ms at 1M rows. Macro benchmark (`oha`) with a stored baseline; criterion micro-benchmarks including h1 vs httparse on the happy path.
- **UI:** Playwright for the core loop (mouse-free), paste-raw round trip, CA-install-after-failure, XSS corpus.
- **Contributor DX:** `just dev`, `just test`, `just fuzz-smoke`; devcontainer; Rust contributors never touch the UI and vice versa; ADRs in `docs/adr/`; hours in `docs/log.md`; the first batch of ten good-first-issues at the alpha.1 tag.

## 14. Roadmap (dated at 12 h/week; every milestone has hours and a definition of done)

**Checkpoint rule:** at 50 % of a milestone's planned hours, if fewer than 50 % of its DoD items are green, apply that milestone's cut list in order and record an ADR. The DoD never slips to meet the date; the cut list does. Cut items ship as point releases, never silently dropped.

### v0.1.0 — core loop. ~300 h, ~24 weeks, target 2027-Q2. Segment: bug bounty hunters and creators (sidecar beside Burp).
Pre-releases every 6 weeks: **0.1.0-alpha.1** (proxy + history + durability), **alpha.2** (repeater + intercept), **beta** (match-and-replace, decoder, browser launcher, docs), **0.1.0**.
- **Non-cuttable core:** core + h1 + client + ca + HTTP/1.1 MITM proxy + store/writer/journal/audit + REST/WS API + history view + repeater (assisted and raw) + scope model and import + the DoD items + security gates + fidelity corpus + release pipeline + CI Linux/macOS + docs site with CA-install guide.
- **Ordered cut list:** Firefox profile support → tags/notes → decoder pane → copy-as-curl → response intercept → match-and-replace → `crosswire browser` (Chromium).
- **Definition of done:** golden HTTP/1 suite green on a corpus of ≥ 200 captures covering every `DiagnosticCode`; kill -9 test green as specified in §13; RSS < 500 MB at 1M synthetic exchanges; install-to-first-intercept < 2 min scripted on tier-1; all §13 security gates green; one-command release pipeline green; `docs/format.md` and the `Message` schema published.

### v0.2a — ~200 h, target 2027-Q4. Segment: students/CTF and heavier hunters.
Intruder with payload sets and grep/extract columns; FTS content search; comparer; HTTP/2 upstream via `h2` adapter (`normalized`); HAR import; Windows tier 1 with DPAPI/ACL tests. In the first 4 weeks: publish `crosswire-core`, `crosswire-h1`, `crosswire-ca` 0.1 (golden + fuzz-smoke green, external review ADR recorded), r/rust + This Week in Rust post, NLnet NGI0 Commons application.
**DoD:** intruder attack types all tested; FTS at 1M rows within query targets; h2 adapter golden suite; Windows CI blocking; crates published.

### v0.2b — ~150 h, target 2028-Q1. Segment: AppSec engineers.
Full query language; CLI (`send`, `replay`, `import`, `export`, `project`); site map / host tree; Burp XML import; out-of-band callback client (opt-in, server configurable); read-only MCP over stdio (§11.8); Show HN, r/netsec, r/bugbounty.
**DoD:** CLI byte-exact replay test from CI; query parser fuzzed; MCP approval test green.

### v0.3 — ~250 h, target 2028-Q3.
Multi-role replay-and-diff; session rules; WebSocket history and replay (`crosswire-ws`); JS plugins (`rquickjs`, typed SDK, §11.5); passive checks; `crosswire-h2` client codec and raw HTTP/2 editing (`decoded-ordered`); staged-write timing tests; `crosswire-client` and `crosswire-h2` published; second `TlsBackend` evaluated. Two co-maintainers listed in MAINTAINERS.md (pipeline in §15).
**DoD:** h2 golden suite frame-for-frame; plugin sandbox tests; multi-role diff on a seeded app.

### v0.4 — ~250 h, target 2029-Q1. Segment: small firms. Go/no-go: at least one small firm committed to piloting team mode before the team-authz RFC is implemented.
Free team mode (central server owns the store; clients proxy locally and stream to it; ULIDs + author; resumable cursors; team-authz RFC first); reporting (Markdown/JSON/SARIF); browser-side HTTP/2.
**DoD:** 5 concurrent users on a seeded project; audit log shipped; RFC implemented as written.

### v0.5+ — HTTP/3, WASM plugins, active scanner framework with community checks, Tauri desktop app, encryption at rest (ADR).

### v1.0 criteria
Frozen feature checklist met; passes the §2.3 fidelity set that normalising tools fail; RSS ceiling at 1M exchanges; zero loss of committed exchanges and zero corruption under kill -9; 5+ concurrent team users; API v1 and store format frozen; third-party security review of proxy + API completed. Adoption metrics without telemetry: binstall/Homebrew counts, external PRs, public write-ups, plugin count.

**Leading indicators and what a miss means:** v0.1.0 success = ≥ 3 weekly-active early-access creators at +6 weeks; v0.2 = first external PR and one public write-up; v0.4 = the pilot firm. A missed gate pauses the app roadmap and shifts the budget to the libraries.

## 15. Open source and community
- **Licence:** Apache-2.0 OR MIT (dual).
- **Governance (GOVERNANCE.md):** BDFL until two maintainers exist, then lazy consensus with a maintainer vote on RFCs (14-day timebox). Contributor ladder: triager → committer → maintainer by vote. crates.io ownership via a GitHub team with ≥ 2 members before the first publish; a named backup org owner and second crates.io owner from day one; domain on auto-renew. Issue/PR templates and labels. **Co-maintainer pipeline:** good-first-issues per crate, a "contributor of the month" mention in release notes, direct invitations after three merged PRs; goal of two co-maintainers by v0.3.
- **Maintainer load (from v0.2):** 8 h build / 4 h community; SLAs in CONTRIBUTING: issue first response 7 days, PR review 14 days, security acknowledgement 72 h.
- **Release cadence:** a tagged release or pre-release every 4–6 weeks from the first commit; the maintainer uses the tool in daily work.
- **Archival plan:** if six months pass without a tag, the backup owner adds an "unmaintained" banner, pins the last release, and hands the org to a volunteer with 3+ merged PRs or archives it; credentials are escrowed so a final release can be cut.
- **Funding:** GitHub Sponsors and Open Collective (fiscal host: Open Source Collective) from v0.1; NLnet NGI0 Commons in the first call after `crosswire-h1` 0.1 (check the calendar now); Sovereign Tech Agency and Alpha-Omega possible for the libraries if they gain dependents, not planned. What funding buys: signing, a third-party security review before 1.0, a contracted UI sprint, maintainer time. Funding never gates the roadmap.

**Launch table**

| When | What | Success signal |
|---|---|---|
| v0.1.0 | 90-second video built from the §2.3 corpus; early access for 5–10 named bug bounty creators; "Coming from Burp" guide | ≥ 3 weekly-active at +6 weeks |
| v0.1.0 + 4 weeks | `crosswire-h1` / `core` / `ca` 0.1 on crates.io; r/rust; This Week in Rust; NLnet application | first external dependent or PR |
| v0.2b | Show HN; r/netsec; r/bugbounty | one public write-up |
| v0.3–v0.4 | conference demo (CFP dates tracked on the roadmap) | pilot firm for v0.4 |

## 16. Risks
- **Budget:** the 60 LoC/h assumption may be optimistic; the checkpoint rule and cut lists are the control; hours are logged.
- **Scope creep:** v0.1 frozen; cut list pre-written.
- **HTTP/2 complexity:** schema fixed now (§6.1); code staged v0.2a → v0.4.
- **"80 % then abandoned":** cadence, per-milestone DoD, co-maintainer pipeline, archival plan.
- **Caido or OpenTRAP close the gap:** openness, free team mode and the libraries; a competitor adopting the crates counts as a win (§2.1).
- **Bus factor:** modular crates, docs, backup owner, escrowed credentials.
- **Tool security:** §11; triage burnout mitigated by the SLAs and the SECURITY.md process.
- **Recurring costs:** Apple signing (~$99/yr), domain; paid from Sponsors; nothing else recurring.
- **Name:** GitHub org, domain, USPTO/EUIPO check; short trademark policy.
- **Misuse:** the tool is for authorized testing; docs say so; strict scope mode exists for shared deployments.

## 17. Out of scope for v0.1.0
Intruder, comparer, full query language, FTS, CLI, HTTP/2, HTTP/3, WebSocket editing, plugins, MCP, team mode, site map, HAR/Burp import (waits for v0.2a because the `Message` schema must be proven on live captures first), out-of-band callbacks, session rules, passive checks, multi-role, encryption at rest (ADR, v0.3+), exports beyond copy-as-curl, transparent/invisible proxy mode (decision deferred to v0.3 ADR), AI features, desktop wrapper.
