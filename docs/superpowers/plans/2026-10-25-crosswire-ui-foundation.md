# Crosswire Web UI 5a: Foundation + History Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a Svelte SPA, embedded in the `crosswire` binary, that authenticates to the Plan 4 loopback API, streams live events, and browses recorded history down to byte-exact request/response inspection.

**Architecture:** A plain client-rendered Vite + Svelte 5 (runes) + TypeScript SPA in a new top-level `ui/` directory, built to a bundle committed under `crates/crosswire-server/assets/ui/` and embedded with `rust-embed`. `crosswire-server` serves that bundle (ungated, with a strict UI CSP + per-response nonce) alongside the existing gated `/api/v1` router; non-`/api/v1` paths are the static-asset unauthenticated class. The SPA consumes the API through a typed client aligned to the committed `docs/api/openapi.json`, holds a single WebSocket event store, and renders captured bytes through a shared CodeMirror 6 byte-exact editor (read-only here, editable in Plan 5b).

**Tech Stack:** Vite, Svelte 5 (runes), TypeScript, CodeMirror 6, Vitest, Playwright, `rust-embed` (Rust), axum/tower (existing).

**Spec:** `docs/superpowers/specs/2026-10-25-crosswire-ui-design.md` (frontend architecture; §9.1 is this plan). Binding product authority: `docs/superpowers/specs/2026-10-22-crosswire-design.md` (§7.0 history, §7.2 editor byte model, §11.1 rendering, §11.2 credentials, §12 embedding). Plan 5b (repeater/intercept/scope) is the next plan.

## Global Constraints

- **Rust:** `rust-version = "1.88"`, `unsafe_code = "forbid"`, `missing_docs = "deny"`; published crates depend only on published-or-workspace crates; no new default features. `crosswire-server` is a binary crate; `rust-embed` is a new dependency there — run `cargo deny check` after adding it.
- **`cargo build` stays Node-free:** the embedded bundle is served from the committed `crates/crosswire-server/assets/ui/`; no build step may require Node for a plain `cargo build`/`cargo install`.
- **UI dependency hygiene:** `ui/package.json` pins exact versions; `ui/package-lock.json` is committed. Minimise runtime deps (Svelte, CodeMirror 6 modules, router). Each external script/style is same-origin from the bundle — the CSP forbids external origins.
- **UI CSP (§11.1, verbatim intent):** `default-src 'self'; frame-src http://127.0.0.1:<port>; connect-src 'self' ws://localhost:<port>; object-src 'none'; base-uri 'none'; frame-ancestors 'none'`, plus `X-Content-Type-Options: nosniff` and a per-response nonce wired to CodeMirror. The API never emits `Access-Control-*`.
- **Untrusted content (§11.1):** captured bytes render as text only (CodeMirror / text nodes). The CI lint bans `{@html}`, `innerHTML`, `insertAdjacentHTML`, `srcdoc`, `javascript:` hrefs and `<svelte:element this>` in `ui/src/`.
- **Credentials (§11.2):** the only new unauthenticated routes are the static assets (the bundle files + the `index.html` fallback). Session cookie `cw_sid_<port>` is HttpOnly and never read by JS; the bootstrap token is taken from the URL fragment and cleared with `history.replaceState`; no secret is persisted in `localStorage`/JS.
- **Host/isolation:** the UI is served only when `Host` is `localhost[:port]` (the existing 421 gate covers the rest); same-origin, so no CORS.
- **Commits:** Conventional Commits, ≤72-char subject, a real why-body, the user's git identity (Kalash Poddar <kalashpoddar33@gmail.com>), and **no attribution trailers**. Verify `git log -1 | grep -c -i claude` is 0 after each commit.

## Review Focus

- **Bootstrap token already consumed / page reloaded** (§11.2): a reload after the fragment was cleared, or a reused single-use token, must land on the auth screen, not a blank page or a crash — Task 4 test.
- **WebSocket drop / `lagged` frame** (spec §8): a dropped connection or a `lagged` resync must refetch the affected view, not silently stop updating or double-count — Task 5 test.
- **Non-Latin-1 bytes and the full `0x00–0xFF` range in the editor** (§7.2): every byte round-trips; pasted non-Latin-1 is UTF-8-encoded and flagged, never silently lost — Task 6 test.
- **A static-asset path colliding with or masking an API route, or an unauthenticated UI path leaking API data** (§11.2): `GET /` is ungated UI, `GET /api/v1/counts` stays `401` unauthenticated — Task 1 test.
- **Captured bytes containing active markup rendered into the DOM** (§11.1): an XSS corpus in history executes nothing — Task 9 test.

---

### Task 0: `ui/` scaffold — Vite + Svelte 5 + TS + CodeMirror, lint, reproducible build

**Files:**
- Create: `ui/package.json`, `ui/package-lock.json`, `ui/vite.config.ts`, `ui/tsconfig.json`, `ui/svelte.config.js`, `ui/.gitignore` (ignores `dist/` and `node_modules/`), `ui/eslint.config.js`, `ui/playwright.config.ts`, `ui/index.html`, `ui/src/main.ts`, `ui/src/App.svelte`, `ui/src/app.css`
- Test: `ui/src/smoke.test.ts` (Vitest)

**Interfaces:**
- Produces: a buildable SPA. `npm run build` → `ui/dist/` (static files); `npm run lint`, `npm run test` (Vitest), `npm run dev`, `npm run e2e` (Playwright). Vite configured for **reproducible output** (no content-hash variance beyond input, no timestamps) so the committed bundle is byte-stable across runs, and with a **dev proxy** forwarding `/api` and the `/api/v1/events` WebSocket to a running `crosswire serve` (dev only). The `eslint.config.js` includes the §11.1 ban rules as errors over `ui/src/`; `playwright.config.ts` is the shared harness later tasks' `.spec.ts` gates run under.

- [ ] **Step 1: Write the failing test** — `ui/src/smoke.test.ts`: mounts `App` and asserts it renders a root element with a known test id (`data-testid="app-root"`).
- [ ] **Step 2: Run** `npm --prefix ui test` → FAIL (no deps / no App yet).
- [ ] **Step 3: Scaffold** the Vite+Svelte 5+TS project with pinned exact versions; add CodeMirror 6 packages (`@codemirror/state`, `@codemirror/view`, `@codemirror/commands`); configure `vite.config.ts` for reproducible builds and the dev proxy (`/api` + the events WS → a running `crosswire serve`); set up Vitest and Playwright (`playwright.config.ts`, `npm run e2e`); write a minimal `App.svelte` rendering `<div data-testid="app-root">`; add the eslint flat config with rules banning `{@html}`/`innerHTML`/`insertAdjacentHTML`/`srcdoc`/`javascript:`/`<svelte:element this>`.
- [ ] **Step 4: Run** `npm --prefix ui test` → PASS; `npm --prefix ui run build` → `ui/dist/` exists; `npm --prefix ui run lint` → clean.
- [ ] **Step 5: Commit** `feat(ui): scaffold the Svelte SPA toolchain with reproducible builds and the no-innerHTML lint`.

---

### Task 1: Embed and serve the bundle; UI CSP; static-asset unauth class

**Files:**
- Create: `crates/crosswire-server/src/ui.rs` (embed + serve + CSP), `crates/crosswire-server/assets/ui/.gitkeep` (the committed bundle lands here)
- Modify: `crates/crosswire-server/Cargo.toml` (add `rust-embed`), `crates/crosswire-server/src/serve.rs` (compose the UI fallback into the served router), `crates/crosswire-api/src/auth.rs:171` (`is_unauthenticated` — treat any non-`/api/v1` path as a static asset), `crates/crosswire-api/src/app.rs` (expose a way to attach a fallback service, or document that the server composes it), `justfile` (add `ui-build`)
- Test: `crates/crosswire-server/tests/ui.rs`

**Interfaces:**
- Consumes: `crosswire_api::router(state) -> axum::Router` (the gated API, `app.rs:184`); `is_unauthenticated(path: &str) -> bool` (`auth.rs:171`).
- Produces: `crosswire_server::ui::service() -> <a tower service>` serving embedded `assets/ui/` with correct content-types and the `index.html` fallback for unknown non-`/api/v1` paths; a CSP + nonce layer applied to UI responses. The server's served router = the API router with the UI service as its fallback. `is_unauthenticated` returns `true` for any path not starting with `/api/v1` (static assets), keeping every `/api/v1` route gated exactly as before.

- [ ] **Step 1: Write the failing test** — `tests/ui.rs`: with a built bundle present, `GET /` (Host `localhost`) returns `200`, an HTML body, and a `Content-Security-Policy` header containing `default-src 'self'`; `GET /assets/<a bundle file>` returns `200` with its correct content-type; `GET /api/v1/counts` with no credential still returns `401` (`missing_credential`); `GET /nope` (unknown UI path) returns the `index.html` fallback with `200`.
- [ ] **Step 2: Run** `cargo test -p crosswire-server --test ui` → FAIL (no serving yet).
- [ ] **Step 3: Implement** `ui.rs` (rust-embed the committed dir; a fallback service; the CSP+nonce layer per the Global Constraints value); modify `is_unauthenticated` so non-`/api/v1` paths are the static class; compose the fallback in `serve.rs`. `just ui-build` runs the Node build and copies `ui/dist/` → `crates/crosswire-server/assets/ui/`.
- [ ] **Step 4: Run** `cargo test -p crosswire-server --test ui` → PASS; `cargo deny check` → clean (rust-embed introduced no advisory).
- [ ] **Step 5: Commit** `feat(server): embed and serve the UI bundle with a strict CSP and static-asset auth class`.

---

### Task 2: CI — Node build, `ui-dist` drift check, lint and test wiring

**Files:**
- Modify: `.github/workflows/ci.yml` (Node setup on the check job; `ui-lint`/`ui-test`/drift steps), `justfile` (add `ui-lint`, `ui-test`, `ui-dist-check`)
- Test: the drift-check recipe itself is the test (fails on a stale bundle, passes when current).

**Interfaces:**
- Consumes: `just ui-build` (Task 1).
- Produces: `just ui-dist-check` — runs the Node build into a temp dir and fails if it differs from the committed `crates/crosswire-server/assets/ui/` (the `openapi_snapshot_is_current` discipline). CI runs `ui-lint`, `ui-test`, and `ui-dist-check` on the ubuntu check job (Node available; the image already has a browser for Playwright later).

- [ ] **Step 1: Write the failing check** — add `just ui-dist-check`; prove it FAILS when `assets/ui/` is deleted/altered and PASSES when rebuilt from the current `ui/` sources.
- [ ] **Step 2: Run** `just ui-dist-check` against a deliberately stale bundle → FAIL as expected; against a fresh one → PASS.
- [ ] **Step 3: Implement** the CI steps: a `setup-node` step pinned by SHA, `npm ci` in `ui/`, then `just ui-lint`, `just ui-test`, `just ui-dist-check`, gated to ubuntu. Keep `cargo build`/`just test` Node-free.
- [ ] **Step 4: Run** `just ui-lint && just ui-test && just ui-dist-check` locally → all green.
- [ ] **Step 5: Commit** `ci: build, lint, test and drift-check the UI bundle on the ubuntu job`.

---

### Task 3: Typed API client + error-envelope handling, aligned to `openapi.json`

**Files:**
- Create: `ui/src/lib/api/client.ts`, `ui/src/lib/api/types.ts`, `ui/src/lib/api/errors.ts`
- Test: `ui/src/lib/api/client.test.ts`

**Interfaces:**
- Produces: `apiGet<T>(path, params?) -> Promise<T>`, `apiSend<T>(method, path, body?) -> Promise<T>` (both attaching `Sec-Fetch-Site: same-origin`, `credentials: 'same-origin'`); `ApiError { code: string; message: string; status: number }` parsed from `{ "error": { "code", "message" } }`; a `UnauthorizedError` subtype for `401`. `types.ts` mirrors the committed `docs/api/openapi.json` schemas (`ExchangeRow`, `Page<T>`, `Counts`, `Scope`, `SendOutcome`, `HeldSummary`, `Event`, …); a build/test step checks the hand-written types against the document (parse `openapi.json`, assert every referenced schema name exists).

- [ ] **Step 1: Write the failing test** — `client.test.ts`: a mocked `fetch` returning the error envelope → `apiGet` rejects with an `ApiError` carrying `code`/`message`/`status`; a `401` → `UnauthorizedError`; a success → the parsed body; a `types-match` test loads `../../../docs/api/openapi.json` and asserts each schema name used in `types.ts` is present.
- [ ] **Step 2: Run** `npm --prefix ui test` → FAIL.
- [ ] **Step 3: Implement** the client, the error types, and the `types.ts` mirror.
- [ ] **Step 4: Run** `npm --prefix ui test` → PASS.
- [ ] **Step 5: Commit** `feat(ui): typed API client with the stable error envelope, aligned to openapi.json`.

---

### Task 4: Bootstrap → session auth flow + auth screen

**Files:**
- Create: `ui/src/lib/stores/session.svelte.ts`, `ui/src/routes/Auth.svelte`
- Modify: `ui/src/App.svelte` (gate the app on session; render `Auth` when unauthenticated)
- Test: `ui/src/lib/stores/session.test.ts`, `ui/tests/auth.spec.ts` (Playwright)

**Interfaces:**
- Consumes: `apiSend` (Task 3).
- Produces: `session` store with `$state` `status: 'unknown' | 'authenticated' | 'unauthenticated'` and `bootstrapFromFragment()` (reads `#bootstrap=<token>`, `POST /api/v1/session`, on success clears the fragment with `history.replaceState` and sets `authenticated`), `submitToken(raw)` (the paste path), and `probe()` (a cheap authenticated GET that sets `unauthenticated` on `401`). No token is stored anywhere after the call.

- [ ] **Step 1: Write the failing tests** — `session.test.ts`: `bootstrapFromFragment` with a valid token sets `authenticated` and the location fragment is emptied; a reused/invalid token sets `unauthenticated`; `probe` maps `401` to `unauthenticated`. `auth.spec.ts`: opening the UI with `#bootstrap=<token>` reaches the app and the URL no longer shows the token; opening without one shows the auth screen; pasting a valid token enters the app.
- [ ] **Step 2: Run** both → FAIL.
- [ ] **Step 3: Implement** the store and `Auth.svelte`; wire `App.svelte` to show `Auth` unless `authenticated`.
- [ ] **Step 4: Run** both → PASS.
- [ ] **Step 5: Commit** `feat(ui): bootstrap-to-cookie session flow with a fragment-clearing auth screen`.

---

### Task 5: Live event-stream store (WebSocket) with `lagged` handling

**Files:**
- Create: `ui/src/lib/stores/events.svelte.ts`
- Test: `ui/src/lib/stores/events.test.ts`

**Interfaces:**
- Consumes: the session being `authenticated` (Task 4).
- Produces: `events` store that opens one WebSocket to `/api/v1/events` (cookie auth, same-origin), exposes `$state` of the latest events and a `subscribe(onEvent, onLagged)` registration; on a `{"type":"lagged"}` frame it invokes `onLagged` (consumers refetch) rather than assuming continuity; it reconnects with backoff on drop and re-announces `lagged` to subscribers.

- [ ] **Step 1: Write the failing test** — `events.test.ts` with a mock WebSocket: a `received`/`committed` frame reaches `onEvent`; a `lagged` frame invokes `onLagged`; a socket close triggers a reconnect attempt and, on reopen, a `lagged` signal to subscribers.
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3: Implement** the store (a thin wrapper over `WebSocket`; inject the socket factory so the test can mock it).
- [ ] **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** `feat(ui): live event-stream store with reconnect and lagged resync`.

---

### Task 6: Byte-exact editor component (Latin-1 + hex pane), read-only

**Files:**
- Create: `ui/src/lib/editor/encoding.ts`, `ui/src/lib/editor/ByteEditor.svelte`, `ui/src/lib/editor/HexPane.svelte`
- Test: `ui/src/lib/editor/encoding.test.ts`, `ui/src/lib/editor/byte-editor.spec.ts` (Playwright)

**Interfaces:**
- Produces: `bytesToLatin1(bytes: Uint8Array) -> string` and `latin1ToBytes(s: string) -> Uint8Array` (exact inverse over `0x00–0xFF`); `encodePastedText(s: string) -> { bytes: Uint8Array; nonLatin1: boolean }` (UTF-8-encode, flag when any char is outside Latin-1); `ByteEditor` (a Svelte component over CodeMirror 6 holding bytes through the Latin-1 mapping, non-printables shown as visible placeholders, a `readonly` prop (true here), a bound `bytes` value, a paste-chip surfaced when non-Latin-1 text is pasted) with `HexPane` showing the same bytes.
- Consumes: CodeMirror 6 (Task 0).

- [ ] **Step 1: Write the failing tests** — `encoding.test.ts`: for every byte `0x00–0xFF`, `latin1ToBytes(bytesToLatin1([b]))` equals `[b]`; a full-range buffer round-trips; `encodePastedText("€")` yields the UTF-8 bytes and `nonLatin1 = true`, while `"abc"` yields `nonLatin1 = false`. `byte-editor.spec.ts`: a captured buffer containing all 256 byte values renders, the hex pane matches, and reading the editor's `bytes` back equals the input (the §7.2 golden, read-only direction).
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3: Implement** `encoding.ts`, then `ByteEditor`/`HexPane` (a CodeMirror document model that never routes bytes through lossy string ops; placeholders via decorations, not DOM HTML).
- [ ] **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** `feat(ui): byte-exact CodeMirror editor with Latin-1 round-trip and a hex pane`.

---

### Task 7: App shell, status bar, and the Burp-compatible keymap foundation

**Files:**
- Create: `ui/src/lib/keymap.ts`, `ui/src/lib/components/Shell.svelte`, `ui/src/lib/components/StatusBar.svelte`, `ui/src/lib/router.ts`
- Modify: `ui/src/App.svelte` (mount `Shell` + the router when authenticated)
- Test: `ui/src/lib/keymap.test.ts`

**Interfaces:**
- Consumes: `session` (Task 4), `events` + `apiGet` for the status bar's counts/scope (Tasks 3, 5).
- Produces: `keymap` — a central, remappable dispatch table mapping named actions (`send-to-repeater`, `send`, `forward`, `drop`, `toggle-intercept`, `decode-selection`, `tab-next`, `tab-prev`) to key chords with the Burp-compatible defaults, plus `register(action, handler)` and `remap(action, chord)`; a tiny history-based `router` (path → view); `Shell` (layout + `StatusBar` showing proxy/API addresses, intercept on/off, scope state and counts from `GET /counts`).
- The mouse-free-core-loop gate (Plan 5b) depends on this dispatch being central, not per-view.

- [ ] **Step 1: Write the failing test** — `keymap.test.ts`: the default table resolves the Burp chords for each named action; `remap` changes a binding and `register`ed handlers fire on dispatch; an unbound chord is a no-op.
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3: Implement** `keymap.ts`, `router.ts`, `Shell.svelte`, `StatusBar.svelte`; mount in `App.svelte`.
- [ ] **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** `feat(ui): app shell, status bar and the remappable Burp keymap foundation`.

---

### Task 8: History view — filters, list, keyset paging, detail

**Files:**
- Create: `ui/src/lib/stores/history.svelte.ts`, `ui/src/routes/History.svelte`, `ui/src/lib/components/ExchangeDetail.svelte`
- Modify: `ui/src/lib/router.ts` (route `/` → History)
- Test: `ui/src/lib/stores/history.test.ts`, `ui/tests/history.spec.ts` (Playwright)

**Interfaces:**
- Consumes: `apiGet` (Task 3), `events` (Task 5, to append `received`/`committed` rows and mark uncommitted), `ByteEditor` read-only (Task 6).
- Produces: `history` store (`$state` rows, active `ExchangeFilter` params, keyset cursor, `loadMore()`, `applyFilter()`, and an `onLagged`→refetch hook); `History.svelte` (the §7.0 columns that alpha.2 supports, sortable, multi-select, filter inputs, uncommitted rows shown distinctly); `ExchangeDetail` (request/response heads+bodies via `ByteEditor` read-only, with `GET /exchanges/{id}` and `GET /exchanges/{id}/body/{side}`, fidelity/diagnostics and the truncation indicator surfaced).

- [ ] **Step 1: Write the failing tests** — `history.test.ts`: `applyFilter` builds the right query params; `loadMore` advances the keyset cursor and appends; a `committed` event flips a row's uncommitted flag; `onLagged` triggers a refetch. `history.spec.ts`: the list loads against a seeded project, a host filter narrows it, selecting a row opens the detail, and the detail shows the response bytes through the editor.
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3: Implement** the store, `History.svelte`, and `ExchangeDetail.svelte`.
- [ ] **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** `feat(ui): history view with filters, keyset paging and byte-exact detail`.

---

### Task 9: XSS-corpus text-rendering gate

**Files:**
- Create: `ui/tests/xss-corpus.spec.ts`, `ui/tests/fixtures/xss-corpus.ts` (the payload set)
- Test: the spec is the test.

**Interfaces:**
- Consumes: the history view (Task 8) and the editor (Task 6) rendering seeded exchanges.
- Produces: a Playwright gate that seeds a project (or mocks the API) with exchanges whose heads/bodies carry an XSS corpus (`<script>`, `<img onerror>`, `javascript:` href, meta refresh, SVG `<use>`, event-handler attributes, …), opens them in history + detail, and asserts nothing executes: no dialog, no navigation away, and no network request beyond the API origin. A page-level listener fails the test on any `dialog`/unexpected request.

- [ ] **Step 1: Write the failing test** — `xss-corpus.spec.ts` with the fixtures; initially point one case at a deliberately unsafe render to confirm the harness *can* fail, then remove that case.
- [ ] **Step 2: Run** → the harness catches the unsafe case (proving it works), then PASS once only safe rendering remains.
- [ ] **Step 3: Implement/confirm** safe rendering (it should already hold from Tasks 6/8 — text nodes / CodeMirror only); fix any leak the gate finds.
- [ ] **Step 4: Run** → PASS; `just ui-lint` still clean (no `{@html}`).
- [ ] **Step 5: Commit** `test(ui): XSS corpus gate proves captured bytes never execute in history`.

---

### Task 10: Install-to-first-capture through the UI (< 2 min) Playwright gate

**Files:**
- Create: `ui/tests/first-capture-ui.spec.ts`
- Modify: `justfile` (a recipe that launches `serve`, builds/embeds the UI, and runs this spec), `.github/workflows/ci.yml` (run it on ubuntu after the existing `first-capture`)
- Test: the spec is the test.

**Interfaces:**
- Consumes: the whole 5a stack — `serve` serving the embedded UI, the bootstrap flow (Task 4), the event store (Task 5), the history view (Task 8). Reuse the Plan 4 server-test spawn pattern (`crates/crosswire-server/tests/cli.rs`) and the `crosswire browser` launcher / an in-process capture, mirroring `scripts/first-capture.sh`.
- Produces: a gate that starts `serve` with an embedded UI build, opens the UI with the bootstrap fragment, drives a capture (through the proxy), and asserts the exchange appears in the history view within 120 s — the §12 DoD item exercised through the UI.

- [ ] **Step 1: Write the failing test** — `first-capture-ui.spec.ts`: launch `serve` (ephemeral ports, `--bootstrap-file`), parse the bootstrap URL, open the UI, cause one capture, assert it shows in history under the time budget.
- [ ] **Step 2: Run** → FAIL (until the flow is wired end to end).
- [ ] **Step 3: Implement** the recipe and any glue (seed/trigger the capture the way `scripts/first-capture.sh` does; a browser is gated behind `CROSSWIRE_BROWSER`/CI as in Plan 3/4).
- [ ] **Step 4: Run** locally with a browser → PASS; confirm the CI step is ubuntu-gated and ordered after `first-capture`.
- [ ] **Step 5: Commit** `test(ui): install-to-first-capture through the UI under two minutes`.

---
