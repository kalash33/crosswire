# Crosswire: design spec

Date: 2026-10-22. Status: draft for judge panel.
Working name: **Crosswire** (free on crates.io as of today).

## 1. What it is

An open-source web application security testing platform for authorized testing, written in Rust: an intercepting proxy, request history, repeater, intruder, and, later, a scanner, plugins, team mode and an agent API. It is an open alternative to Burp Suite Pro and Caido.

It ships in two forms:
1. **The app:** a headless Rust server plus a web UI, with a desktop wrapper.
2. **The libraries:** public crates that others can build on. The foundation is `crosswire-wire`, a **byte-exact HTTP library for security tooling**. It is a new primitive that the Rust ecosystem lacks, because `hyper` and `h2` normalise traffic by design.

**Sustainability:** pure community open source. There are no paid tiers and no closed parts.

## 2. Why it exists (evidence summary)

- **Burp Suite Pro** costs $499 per user per year, and AI costs extra through credits that expire and can't be pooled. It has about 88,000 customers. Users repeatedly complain that it uses a lot of memory, that project files corrupt or grow to many gigabytes, that there is no native team collaboration, and that the UI is dated.
- **Caido** is a closed-source Rust app costing $200 per year. It gained about 36,000 users in 2025, which shows that people will switch. Its free tier allows 2 projects and 3 plugins, and team mode is paid. HTTP/2 proxying is still incomplete (issue #439 has been open since 2023), it has no HTTP/3, and its scanner is only a community plugin.
- **ZAP** is open source but written in Java with a dated UI, and its roadmap is now influenced by Checkmarx.
- **Hetty** (12.5k stars) has made no release since 2022.
- **mitmproxy** is a developer tool rather than a pentest suite.
- **OpenTRAP** launched on 2026-10-19 and is built on ZAP and Java.
- **No open-source, Rust-based tool of this class exists.**

## 3. Users, in priority order

1. **Bug bounty hunters.** They are price sensitive, adopt new tools early and drive word of mouth.
2. **Small pentest firms of 3–30 testers.** They need collaboration and reporting.
3. **Students, CTF players and people in price-sensitive regions.**
4. **AppSec engineers.** They need headless mode, CI use and API testing.
5. **Large consultancies.** They come later, since they are locked into Burp for procurement reasons.

## 4. Product principles (the bar for "extremely good")

1. **Fidelity.** What you type is exactly what is sent, byte for byte. That includes malformed requests, duplicate headers, odd line endings and header order. What came back is shown exactly as received.
2. **Never lose work.** Storage is crash-safe, there is no single project file that can corrupt, and the history is append-only.
3. **Fast at scale.** The UI stays responsive with 1M+ requests and 50 GB of bodies, and memory use is bounded no matter how large the history gets.
4. **API-first.** Anything the UI can do, the API can do. The UI is just one client, alongside the CLI, plugins, the MCP server and CI.
5. **Free means everything.** Team mode, unlimited projects and plugins are never gated.
6. **Safe by default.**
   - The server binds to localhost and requires an auth token.
   - Out-of-scope requests are blocked.
   - Plugins run in a sandbox with explicit permissions.
   - The CA key is generated per install and never leaves the machine.
7. **Library quality.** The crates are documented, semver-disciplined, fuzzed and usable without the app.

## 5. Architecture

```
            ┌──────────── clients ─────────────┐
 Browser ─► │ web UI (Svelte/TS)  CLI  MCP  CI │
 traffic    └───────────────┬──────────────────┘
   │                        │ REST + WebSocket events (OpenAPI)
   ▼                        ▼
┌──────────────────── crosswire-server (binary) ────────────────┐
│ crosswire-proxy  ─►  crosswire-store  ◄─  crosswire-tools     │
│   (MITM engine)      (SQLite + blobs)     (repeater/intruder) │
│        │                    ▲                                 │
│        ▼                    │ crosswire-query (filter lang)   │
│ crosswire-wire  ◄───────────┴──── crosswire-plugins (later)   │
│ (byte-exact HTTP/1, HTTP/2, TLS, WebSocket)                   │
└───────────────────────────────────────────────────────────────┘
```

Cargo workspace crates:

| Crate | Kind | Responsibility |
|---|---|---|
| `crosswire-wire` | public lib | Byte-exact HTTP message model, tolerant parser and exact writer for HTTP/1.x. Frame-level HTTP/2 with its own HPACK encoder that can produce spec-violating frames on request. TLS connector over rustls, with an optional `openssl` feature for legacy servers. WebSocket frame codec. Connection pooling. No proxy logic. |
| `crosswire-ca` | public lib | Local CA creation, protected key storage, per-host leaf certificates via `rcgen` with an LRU cache, and export for browser/OS trust. |
| `crosswire-proxy` | public lib | MITM engine: HTTP proxy with CONNECT, TLS interception with ALPN negotiation (h2/http1.1), upstream HTTP/2, WebSocket upgrade capture, intercept/hold/modify queue, scope rules, upstream proxy chaining (HTTP/SOCKS5). |
| `crosswire-store` | public lib | Project storage. SQLite in WAL mode holds metadata. A content-addressed, zstd-compressed blob store holds bodies, deduplicated by SHA-256. A single writer task does batched inserts. FTS5 indexes searchable text. Schema migrations. Each project is a directory, not one file. |
| `crosswire-query` | public lib | A small filter language, e.g. `req.method == "POST" and resp.status >= 500 and req.host ~ "api."`. Parser → AST → SQL, with tests. |
| `crosswire-tools` | lib | Repeater: send exact bytes and keep per-tab history. Intruder: positions, attack types (sniper, battering ram, pitchfork, cluster bomb), payload sources, concurrency and rate limits, result grid. Also decoder and comparer. |
| `crosswire-api` | lib | REST endpoints plus a WebSocket event stream. The OpenAPI spec is generated with `utoipa`, and the TS client is generated from that spec. |
| `crosswire-server` | binary | `crosswire serve`. Wires everything together, serves the embedded UI, handles auth tokens, and checks the Host header against DNS-rebinding attacks. |
| `crosswire-cli` | binary | Headless commands: `send`, `replay`, `import har`, `export`, `project ls`. |
| `ui/` | TS app | Svelte + TypeScript. Virtualized tables (1M+ rows), CodeMirror 6 editors with a raw/hex/pretty toggle, keyboard-first. |

**Why REST + OpenAPI instead of GraphQL (Caido's choice).** Every client gets a generated typed client: TS, Python, and Rust itself. The MCP server maps cleanly onto it. Contributors only need one well-known style, and CI and curl users get trivial access.

**Why our own wire layer.**
- `hyper` and `h2` reject or rewrite exactly the traffic testers need: duplicate or ambiguous framing headers, odd header casing and ordering, obsolete line folding, and invalid pseudo-headers.
- `crosswire-wire` keeps the original bytes alongside a parsed view. Editing works on the bytes, and the parsed view is derived from them.
- For proxying, we still rely on mature crates where fidelity isn't needed: `rustls`, `rcgen`, `tokio`, and `quinn` later for HTTP/3.

## 6. Key data flows

1. **Proxying a request.**
   - The browser sends CONNECT to the proxy, which terminates TLS with a leaf certificate.
   - `wire` parses the request and keeps its raw bytes.
   - The scope check runs. If intercept is on, the request is held in the queue until it is released or edited.
   - The request is forwarded upstream through `wire`. The response comes back and is relayed to the browser.
   - The raw exchange plus timing is queued to the `store` writer, which batches it into SQLite and the blobs.
   - An event is pushed to clients and the UI history updates live.
2. **Repeater.**
   - The user edits raw bytes and `wire` sends them exactly.
   - The response is stored as a repeater entry linked back to the original request.
3. **Intruder.**
   - Positions are marked as byte ranges in the raw request.
   - The engine generates requests with bounded concurrency and a rate limit.
   - Results are streamed into the store and the grid. A hard stop and a pause are always available.

## 7. Error handling

- **Network and TLS errors** become stored exchanges with an error state. They are never silently dropped.
- **Parser** never panics. Malformed input produces a "raw only" exchange with diagnostics.
- **Store write failures** apply back-pressure to the proxy and show a clear UI alert. Data in flight is not discarded.
- **Plugin crashes** are isolated per plugin and leave the core unaffected.
- **Every API error** has a stable code and message.

## 8. Security of the tool itself

- The server binds to 127.0.0.1 by default. Remote or team use requires explicit configuration and TLS.
- API tokens: a random token is generated on first run. Team mode adds per-user tokens and roles.
- Host/Origin header checks protect the local web UI from DNS rebinding and cross-site requests.
- The CA private key is stored with file permissions limited to the user, and can optionally use the OS keychain.
- Scope guard: tools refuse to send out-of-scope requests unless the user explicitly overrides it per request.
- Plugins (phase 2) get a capability manifest: network, store read, store write and UI. They are denied anything not declared.

## 9. Testing strategy

- **Golden fidelity tests:** corpora of raw HTTP/1 and HTTP/2 exchanges that must round-trip byte-exactly.
- **Fuzzing:** `cargo-fuzz` on the HTTP/1 parser, the HTTP/2 frame codec, HPACK and the query parser, run in CI on a schedule.
- **Property tests:** `proptest` for parse/serialize round-trips.
- **Integration harness:** in-process test servers (HTTP/1, h2, TLS, WebSocket, and deliberately misbehaving ones) for end-to-end proxy and repeater tests.
- **Performance benchmarks** with criterion:
  - proxy throughput and latency added per request;
  - store ingestion of at least 2k exchanges/s;
  - history query on 1M rows in under 200 ms;
  - memory stays bounded.
- **UI:** Playwright end-to-end tests for the core flows.
- **CI:** Linux, macOS and Windows.

## 10. Roadmap

- **v0.1, the core loop (part-time, ~10–12 weeks):**
  - `wire` (HTTP/1 exact, client side of HTTP/2) and `ca`;
  - `proxy` (HTTP/1 + HTTP/2 upstream, intercept, scope);
  - `store` + `query`;
  - repeater, basic intruder;
  - REST API;
  - web UI (history, intercept, repeater, intruder);
  - CLI basics.
  - **Release:** `crosswire-wire` 0.1 published on its own first.
- **v0.2:** WebSockets (history and replay), JS plugins (`rquickjs`, typed SDK), HAR/Burp XML import, decoder/comparer.
- **v0.3:** team mode (multi-user server, roles, live shared history), MCP server, raw HTTP/2 editing and single-packet race testing.
- **v0.4:** out-of-band callback testing (a self-hostable callback server), multi-role access-control testing (replay as other users and diff the responses), reporting/export (Markdown, JSON, SARIF).
- **v0.5+:** HTTP/3 (`quinn`), WASM plugins, a passive checks engine, an active scanner framework with community checks, and the Tauri desktop app.
- **v1.0 criteria:** the daily workflow is at parity with Caido, there are 10+ community plugins, docs are complete, and the fidelity, fuzzing and performance suites are green.

## 11. Open source and community

- **Licence:** Apache-2.0 OR MIT (dual), which is the Rust norm, for maximum contribution and embedding. AGPL-3.0 was considered; it was rejected because it deters embedding of the libraries.
- **Process:** CONTRIBUTING guide, good-first-issues per crate, architecture docs, an RFC process for plugin API changes, and a code of conduct.
- **Funding:** GitHub Sponsors and Open Collective, plus grant applications to NLnet, the Sovereign Tech Fund and OpenSSF Alpha-Omega.
- **Launch sequence:**
  1. `crosswire-wire`: r/rust and This Week in Rust.
  2. The v0.1 app: Show HN, r/netsec and r/bugbounty.
  3. A demo by bug bounty creators.
  4. A Black Hat Arsenal / DEF CON demo.

## 12. Risks

- **Scope creep before v0.1.** Mitigation: v0.1 is frozen to the core loop above.
- **HTTP/2 fidelity complexity.** Mitigation: ship HTTP/1 exactness first, and use HTTP/2 through a compliant path until the raw HTTP/2 layer is ready.
- **"80% syndrome"** (Hetty's fate). Mitigation: publish a public roadmap, make the plugin API the extension point, and court contributors early.
- **Caido closes its gaps.** Mitigation: openness, free team mode, and the libraries are differentiators Caido can't copy without changing its model.
- **Bus factor (part-time maintainer).** Mitigation: a modular crate design, docs and early co-maintainers.

## 13. Out of scope for v0.1

Scanner, plugins, team mode, HTTP/3, WebSocket editing, browser automation and crawling, AI features, and the desktop wrapper.
