# Crosswire web UI: design spec

**Status:** design, 2026-10-25. Focused companion to the v4 design spec
(`docs/superpowers/specs/2026-10-22-crosswire-design.md`); that document is the
binding authority for product behaviour (§7), untrusted-content rendering
(§11.1), listeners and credentials (§11.2) and distribution (§12). This spec
pins the **frontend architecture** those sections leave open, for the alpha.2
UI that drives the Plan 4 backend.

**U1 addendum (2026-10-31):** the visual design, app shell and per-view
layouts were reworked in U1 (design tokens with dark/light themes, a tool rail
and top bar, a Start panel, `/decoder` and `/settings` routes, visible row
actions, the split exchange detail with Headers/Raw/Hex tabs). Where the two
disagree on layout, components or discoverability, follow
`docs/superpowers/specs/2026-10-31-crosswire-u1-proper-app-ui-design.md`; the
architecture, data flow and §11.1 rules here are unchanged.

## 1. What this is

The Crosswire web UI: a Svelte single-page app that drives the alpha.2
REST + WebSocket API (built in Plan 4, `crosswire-api`), served by
`crosswire-server` on the loopback API listener and embedded in the single
binary. It is the operator-facing surface for the core daily loop —
history, repeater, request intercept and scope — on top of the byte-exact
recorder and proxy already shipped.

It is **not** a new product surface: every capability it exposes already
exists as an audited, scoped API route. The UI adds no server authority of
its own; it is a client of the same credentials and gates any other client
uses (§11.2 of the v4 spec).

### 1.1 Scope and the two plans

One spec, executed as two implementation plans so each ships something
testable:

- **Plan 5a — foundation + history.** The frontend toolchain and
  build/embed pipeline, the server's static-asset serving and UI CSP, the
  API client, the bootstrap→cookie auth flow, the live event-stream store,
  the app shell and keymap foundation, the **history view**, and the
  **byte-exact editor component** (built once, shared by 5b). Ships a UI
  that authenticates and browses recorded history.
- **Plan 5b — repeater, intercept, scope.** The **repeater** (assisted and
  raw, drafts, results), **request intercept** (hold/edit/forward/drop/
  forward-all/send-to-repeater, a simple rules editor), and the **scope
  editor** (rules, import, enforce-strict). Ships the full alpha.2 daily
  loop, mouse-free.

### 1.2 Deferred (named, not dropped)

- **Rendered HTML/image previews** and their security apparatus — the
  separate preview host (`127.0.0.1`/`[::1]`), HMAC capability-token URLs,
  the sandboxed iframe, and the **zero-egress preview corpus** (§11.1) —
  are a focused later plan. The UI renders captured bytes as **text only**
  until then.
- Beta tools: match-and-replace UI, decoder/inspector pane, tags/notes UI,
  copy-as-curl, response intercept (v4 §7, beta).
- v0.2a and later: intruder, comparer, FTS search UI, HTTP/2 and WebSocket
  frame views.
- The redaction-golden gate for copy flows (arrives with copy-as-curl).

## 2. Stack and repo layout

- **Stack:** Vite + Svelte 5 (runes) + TypeScript, a plain client-rendered
  SPA (no SSR, no SvelteKit). CodeMirror 6 for every byte/text editor and
  viewer. A minimal history-based client router; the server serves
  `index.html` as the fallback for any non-`/api`, non-asset path so deep
  links work.
- **Why no SvelteKit:** a loopback tool embedded in a binary needs no SSR,
  no server endpoints and no adapter runtime; a static SPA is the smallest
  surface to lock down under the strict CSP and the simplest to embed.
- **Location:** a new top-level `ui/` directory holds all frontend sources
  and its own `package.json`, `vite.config.ts`, `tsconfig.json`,
  `svelte.config.js`, `eslint`/lint config, and `playwright.config.ts`. It
  is not a Cargo crate and not in the workspace; it is a sibling the build
  pipeline consumes.
- **Dependencies:** pinned exact versions; a lockfile (`package-lock.json`)
  is committed. New runtime JS dependencies are limited to Svelte,
  CodeMirror 6 modules, and the router; everything else is justified in the
  plan. `npm audit` (or equivalent) runs in CI as an advisory, mirroring
  `cargo deny`.

## 3. Build, embed, dev workflow, CI

- **Build:** `vite build` emits to `ui/dist/`, which is copied to
  `crates/crosswire-server/assets/ui/` and **committed**. `crosswire-server`
  embeds that directory with `rust-embed` at compile time, so a plain
  `cargo build`/`cargo install` is hermetic and needs no Node toolchain.
- **Drift check:** a CI job (`ui-dist`) runs the Node build and fails if the
  committed `assets/ui/` differs from a fresh build — the same discipline as
  the `openapi_snapshot_is_current` test. The build must be reproducible
  enough for byte-identical output (pinned deps, no timestamps/hashes that
  vary run to run; configure Vite accordingly).
- **Justfile:** `just ui-dev` (Vite dev server), `just ui-build` (build +
  copy into the embed dir), `just ui-test` (Vitest + Playwright), `just
  ui-lint`. `just build`/`just test` gain the UI steps where appropriate;
  the drift check is its own CI step.
- **Dev workflow:** `just ui-dev` runs Vite on its own port and proxies
  `/api` and the `/api/v1/events` WebSocket to a separately-running
  `crosswire serve`. In production the SPA and API share the one loopback
  origin, so no CORS is ever needed (and the API emits none — §11.2).
- **CI:** a Node setup step on the ubuntu job; Vitest + Playwright run
  against the embedded build (Playwright needs the browser already present
  in the CI image, as the existing `browser-test` does). The drift check
  runs on both OSes or at least ubuntu.

## 4. Server integration

Extends Plan 4's `crosswire-api`/`crosswire-server`; adds no new authority.

- **Serving:** the embedded SPA is served by the server. Static assets are
  the unauthenticated route class §11.2 reserved ("static assets join this
  list in Plan 5"): `GET` of the bundle's files and the `index.html`
  fallback need no session, exactly like `health`/`openapi.json`/the CA
  routes. Everything under `/api/v1` keeps its existing auth, scope and
  isolation gates unchanged.
- **Host and isolation:** the UI is served only when `Host` is
  `localhost[:port]` (the existing 421 check covers the rest). The
  cookie-isolation rule (§11.2) already in place — every cookie-auth request
  including the WS upgrade must carry `Sec-Fetch-Site: same-origin` or an
  exact-match `Origin` — is satisfied because the SPA is same-origin with
  the API. No CORS headers, ever.
- **Headers (§11.1):** responses that serve the app carry the UI CSP:
  `default-src 'self'; frame-src http://127.0.0.1:<port>; connect-src 'self'
  ws://localhost:<port>; object-src 'none'; base-uri 'none';
  frame-ancestors 'none'`, plus `X-Content-Type-Options: nosniff` and a
  per-response nonce wired to CodeMirror. (`frame-src` names the future
  preview host; harmless before previews exist.) The exact header
  construction is a server task in 5a.
- **The CA info page** served at `http://crosswire/` and on origin-form
  requests (§11.2) is unchanged and is **not** this SPA — it stays the
  static, data-free page from Plan 3.

## 5. Data flow, auth and state

- **API client:** a thin typed client aligned with the committed
  `docs/api/openapi.json`. Types are generated from or checked against that
  document so the client cannot drift from the server contract. Every
  response path surfaces the stable error envelope
  `{ "error": { "code": "<snake_case>", "message": "..." } }`; a shared
  handler maps `401` to the auth screen and other codes to a toast.
- **Bootstrap → session:** on load the SPA reads `#bootstrap=<token>` from
  the URL fragment, calls `POST /api/v1/session`, and on success clears the
  fragment with `history.replaceState`. The session cookie
  (`cw_sid_<port>`, HttpOnly) is never visible to JS. If there is no token
  and no valid cookie (a `401` from a probe), the app shows a small auth
  screen that accepts a pasted bootstrap URL/token. No secret is ever stored
  in `localStorage` or JS memory beyond the in-flight request.
- **Event stream:** one WebSocket to `/api/v1/events`, authenticated by the
  same-origin cookie (the upgrade carries `Origin`). It feeds a single live
  store; the history and intercept views derive from it with Svelte 5 runes
  (`$state`/`$derived`). The client handles the `lagged` frame on reconnect
  by refetching the affected view rather than assuming continuity.
- **State modules:** small, single-purpose modules — `apiClient`,
  `session`, `events` (the WS store), `history`, `intercept`, `scope`,
  `repeaterTabs` — each owning one slice, each testable in isolation. No
  global mutable singleton beyond these stores.

## 6. Views and components

### 6.1 App shell and keymap (5a)
A pane-based layout (navigation, a main view, a status bar showing proxy/API
addresses, intercept on/off, scope state and counts). The **Burp-compatible
default keymap** is established here as a central, remappable dispatch table
covering send-to-repeater, send, forward/drop, toggle intercept, decode
selection and tab switching; views register handlers against it. The
mouse-free-core-loop gate (5b) depends on this being central, not scattered.

### 6.2 History view (5a)
Columns per v4 §7.0: #, time, method, host, path, status, length, MIME, TLS,
response ms, fidelity, tags, notes. Sortable, multi-select, context menu
(send to repeater, copy URL/raw, add host to scope, tag — the actions whose
API exists in alpha.2; copy-as-curl and tag editing that need beta API are
stubbed or hidden). Keyset pagination against `GET /exchanges` with the
filter params the API exposes. A detail view shows request and response
heads and bodies through the **editor component** in read-only mode, with
raw and text rendering and the fidelity/diagnostics surfaced. Uncommitted
rows are shown distinctly. The streaming content-search box is 0.1.x; 5a
ships the structured filters the API already supports.

### 6.3 Byte-exact editor component (5a, shared)
The single most load-bearing component; built in 5a because both the history
detail view and 5b's repeater/intercept depend on it. Per v4 §7.2:
- Bytes are held and shown through a **1:1 Latin-1 mapping** so every byte
  `0x00–0xFF` round-trips exactly; non-printable bytes render as visible
  placeholders; a **hex pane** shows what the text view cannot.
- Pasted text that is not Latin-1 is UTF-8-encoded and reported as a chip.
- Built on CodeMirror 6 with a custom document model/encoding layer; the
  editor never lets the browser's string handling mangle bytes.
- Golden behaviour (a test, not prose): capture → editor → edit one header →
  emit; body bytes `0x00–0xFF` are unchanged. Partial/timed-out responses
  render received bytes plus a diagnostic banner.

### 6.4 Repeater (5b)
Per v4 §7.2: tabs; a new tab starts in **assisted** mode (Content-Length
update and line-ending normalisation on, shown as chips; the CL chip never
touches a `Transfer-Encoding` message and shows "skipped: TE present");
**raw** is a per-tab toggle showing a diff of what assisted would change;
connection settings (host, port, TLS, SNI separate from the Host header,
fresh vs reused); follow-redirects off/in-scope/always with a cap; the
upstream-cert-override badge on the result header; paste-raw and copy-raw as
first-class flows; TTFB and total time in the result header; drafts and
connection settings autosave (debounced) to the project via the API;
results live in the `repeater` namespace linked to the source exchange. All
of this maps onto the Plan 4 repeater API (`POST /repeater/send`, the
`repeater/tabs` routes, drafts).

### 6.5 Request intercept (5b)
Per v4 §7.3: an **off-by-default** toggle in the status bar; a held-items
list with the oldest focused; actions forward, edit-and-forward, drop,
forward-all, send-to-repeater; the intercept editor is the 6.3 component with
the same chips; held items show "not editable" when the body exceeded the
hold limit. A simple rules editor (field filters: the default
skip-by-extension / skip-out-of-scope rules, editable) maps onto the Plan 4
`intercept` routes and the live `{"type":"intercept","held":n}` events.
Response intercept is beta and out of scope.

### 6.6 Scope editor (5b)
Per v4 §4.6/§4.7: edit the ordered include/exclude rules (host pattern +
optional ports/scheme/path), **import** a pasted program list, and toggle
`enforce_scope_strictly`. Maps onto `GET/PUT /scope`, `POST /scope/import`
and `PUT /settings/enforce_scope_strictly`. Scope state is reflected in the
status bar and the history `scope_state` column.

## 7. Security of untrusted content (§11.1)

- Captured bytes render **as text only** — CodeMirror or text nodes, never
  as markup. A **CI lint** bans `{@html}`, `innerHTML`, `insertAdjacentHTML`,
  `srcdoc`, `javascript:` hrefs and `<svelte:element this>` in the UI
  sources; it fails the build on any hit.
- The UI CSP and per-response nonce (§4 above) are wired so no inline script
  runs without the nonce and no external origin is reachable.
- **XSS-corpus gate (5a):** a Playwright test loads an XSS corpus into
  history (and, in 5b, repeater and intercept) and asserts nothing executes
  — no alert, no navigation, no network beyond the API. This is one of the
  §13 gates Plan 4 deferred to "Plan 5/6"; 5a closes the text-rendering half.
- Rendered previews and the zero-egress corpus remain deferred (§1.2).

## 8. Testing

- **Unit/component:** Vitest for the API client, the stores, the editor's
  byte/encoding layer (the `0x00–0xFF` round-trip is a unit test as well as
  an e2e golden), and the keymap dispatch.
- **End-to-end (Playwright), the gates:**
  - XSS corpus does not execute (5a; extended to repeater/intercept in 5b).
  - **Install-to-first-capture through the UI < 2 min** (5a/5b): launch
    `serve`, open the UI, authenticate, drive a capture, see it in history —
    the §12 DoD item exercised via the UI rather than the API.
  - Paste-raw request → send → byte round-trip (5b).
  - **Mouse-free core loop** (5b): the Burp keymap drives capture → send to
    repeater → edit → send → intercept toggle entirely from the keyboard
    (v4 §7.6).
- **CI:** Vitest and Playwright run on the ubuntu job against the embedded
  build; the `ui-dist` drift check guards the committed bundle.

## 9. Plan breakdown and DoD

### 9.1 Plan 5a — foundation + history
Delivers: the `ui/` toolchain; the build/embed pipeline + `rust-embed`
serving + the static-asset route + UI CSP/nonce on the server; the dev proxy;
the API client + error-envelope handling; the bootstrap→cookie auth flow +
auth screen; the WS event-stream store; the app shell + status bar + keymap
foundation; the history view (filters, detail, raw/text rendering); the
byte-exact editor component (read-only use in history); the no-innerHTML CI
lint; the XSS-corpus text-rendering gate; the Playwright harness +
install-to-first-capture-through-the-UI.
**DoD:** `cargo build` embeds a working UI with no Node; the drift check is
green; a user can bootstrap, browse history and inspect an exchange's bytes;
the XSS corpus does not execute; install-to-first-capture-through-the-UI
passes; `just ui-lint`/Vitest/Playwright green in CI.

### 9.2 Plan 5b — repeater, intercept, scope
Delivers: the repeater (6.4), request intercept (6.5), scope editor (6.6),
the editor component in editable use, the mouse-free-core-loop gate, the
paste-raw round-trip gate, and the XSS corpus extended to repeater/intercept.
**DoD:** the full alpha.2 daily loop works mouse-free; drafts/scope persist
through the API; paste-raw byte round-trip holds; all 5b Playwright gates
green; the alpha.2 mini-DoD UI items (history, repeater, request intercept)
are complete.

## 10. Decisions on record

- One UI spec, two plans (5a foundation + history, 5b repeater/intercept/
  scope); performance/1M-RSS stays a separate later plan.
- Vite + Svelte 5 (runes) + TypeScript, plain SPA, CodeMirror 6, embedded
  via `rust-embed`; no SvelteKit.
- UI bundle committed to `crates/crosswire-server/assets/ui/` with a CI
  drift-check; `cargo build` stays hermetic and Node-free.
- Text-safe rendering + XSS-corpus gate + no-innerHTML lint + UI CSP in
  Plan 5; rendered HTML/image previews, the separate preview host,
  capability-token URLs and the zero-egress gate deferred to a focused
  later plan.
