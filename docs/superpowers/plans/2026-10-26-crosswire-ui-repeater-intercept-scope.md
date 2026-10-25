# Crosswire Web UI 5b: Repeater, Intercept, Scope Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Complete the alpha.2 daily loop in the embedded UI — repeater (assisted/raw), request intercept (hold/edit/forward/drop/forward-all/send-to-repeater), and the scope editor — all drivable mouse-free, on top of the Plan 5a foundation.

**Architecture:** Three new feature slices in the existing Svelte SPA, each a small store + view that drives the already-merged Plan 4 API: `repeater` (`POST /repeater/send`, the `repeater/tabs` draft routes), `intercept` (`GET/PUT /intercept`, `/intercept/enabled`, `/intercept/rules`, `/intercept/forward-all`, `/intercept/held/{id}/forward|drop`, plus the live `{"type":"intercept","held":n}` event), and `scope` (`GET/PUT /scope`, `POST /scope/import`, `PUT /settings/enforce_scope_strictly`). The Plan 5a byte-exact `ByteEditor` becomes **editable** here (request editing in repeater and intercept) with assisted-mode chips; the Plan 5a central keymap gets its action **handlers registered** so the whole loop runs from the keyboard. Captured bytes stay text-only (§11.1); the new surfaces extend the XSS-corpus gate.

**Tech Stack:** Vite, Svelte 5 (runes), TypeScript, CodeMirror 6, Vitest, Playwright — all established in Plan 5a. No new runtime dependencies expected.

**Spec:** `docs/superpowers/specs/2026-10-25-crosswire-ui-design.md` (§6.4 repeater, §6.5 intercept, §6.6 scope, §9.2 is this plan). Binding product authority: `docs/superpowers/specs/2026-10-22-crosswire-design.md` (§7.2 repeater, §7.3 intercept, §4.6/§4.7 scope, §7.6 mouse-free loop, §11.1 rendering, §11.2 credentials). Plan 5a (foundation + history) is merged on `main`.

## Plan 5a interfaces this plan consumes (already on `main`)
- `ui/src/lib/api/client.ts`: `apiGet<T>(path, params?)`, `apiSend<T>(method, path, body?)`, `apiGetBytes(path) -> { bytes: Uint8Array; truncatedAt: number | null }`; `ApiError`/`UnauthorizedError` in `errors.ts`; named request types in `types.ts`.
- `ui/src/lib/editor/ByteEditor.svelte` (currently `readonly` use), `encoding.ts` (`bytesToLatin1`/`latin1ToBytes`/`encodePastedText`), `HexPane.svelte`, `nonprintable.ts`, `csp.ts`.
- `ui/src/lib/stores/events.svelte.ts`: `events.subscribe(onEvent, onLagged)`, the `Event` discriminated union (incl. `{type:'intercept',held}`).
- `ui/src/lib/keymap.ts`: central dispatch table with `register(action, handler)`, `remap(action, chord)`; actions `send-to-repeater`, `send`, `forward`, `drop`, `toggle-intercept`, `decode-selection`, `tab-next`, `tab-prev`. **After the 5a final-review fix, a bound chord only `preventDefault`s and dispatches when a handler is registered** — so registering handlers here is what activates the Burp chords.
- `ui/src/lib/router.svelte.ts`: exported `router` singleton; `ui/src/lib/components/Shell.svelte` (mounts the keydown dispatch + `StatusBar`); `ui/src/lib/components/StatusBar.svelte` (intercept/scope indicators already read `GET /intercept` + `GET /scope`).
- `ui/src/routes/History.svelte`, `ui/src/lib/stores/history.svelte.ts` (the send-to-repeater / add-host-to-scope context actions were stubbed/disabled in 5a and are activated here).

## Plan 4 API shapes this plan drives (openapi leaves several untyped — read the handler for the exact shape, as Plan 5a Task 8 did)
- **Repeater** — `POST /api/v1/repeater/send` body `SendBody { source?: string; host: string; port: number; tls: boolean; sni?: string; verify?: string; raw: string; mode?: 'assisted'|'raw'; follow?: 'off'|'in-scope'|'always'; reuse: boolean; allow_out_of_scope: boolean }` (requires the `send` cap) → `SendOutcomeDto { exchange_id: string; scope_state: string; changes: { op: string; reason: string; start: number; end: number }[]; skipped: string[]; cert_override: boolean; ttfb_ms: number; total_ms: number; redirects: string[] }`. Drafts: `GET /repeater/tabs` → `DraftDto[]` where `DraftDto { tab_id: string; json: unknown; updated_at: number }`; `GET/PUT/DELETE /repeater/tabs/{id}` (PUT body is the tab's opaque `json`). `raw` is the full request bytes as a string (Latin-1); source: `crates/crosswire-api/src/routes/repeater.rs`.
- **Intercept** — `GET /api/v1/intercept` → `InterceptStatus { enabled: boolean; rules: RulesDto; held: HeldDto[] }`; `HeldDto { id: string; host: string; method: string; path: string; size: number; editable: boolean }`; `RulesDto { skip_extensions: string[]; skip_out_of_scope: boolean; overflow: 'forward-newest'|'forward-oldest'; on_timeout: 'forward'|'respond-504' }`. `PUT /intercept/enabled` body `{ enabled: boolean }`; `PUT /intercept/rules` body `RulesDto`; `POST /intercept/held/{id}/forward` body `ForwardBody { head?: string /*base64 std*/; body?: string /*base64 std*/ }`; `POST /intercept/held/{id}/drop`; `POST /intercept/forward-all`. Live event `{type:'intercept',held:number}`. Source: `crates/crosswire-api/src/routes/intercept.rs`.
- **Scope** — `GET /api/v1/scope` → the `crosswire_scope::Scope` document (ordered include/exclude rules; read its shape from `crates/crosswire-scope/src/`); `PUT /scope` takes that JSON and **requires the `admin` cap**, returns `422 scope_invalid` if it does not compile (the live scope is left untouched on error); `POST /scope/import` body `{ text: string }` → a parsed `Scope` **without saving** (the UI then PUTs it); `PUT /settings/enforce_scope_strictly` body `{ enabled: boolean }`. Source: `crates/crosswire-api/src/routes/scope.rs`.

## Global Constraints

- **Rust:** no Rust changes are expected in this plan (the API is complete on `main`). If a server change proves necessary, it inherits Plan 5a's constraints (`rust-version = "1.88"`, `unsafe_code = "forbid"`, `missing_docs = "deny"`, rustls ring-only, `cargo deny` clean) and must keep `cargo build` Node-free.
- **Committed bundle (Ruling B, carried from 5a):** every task that changes `ui/src` runs `just ui-build` and commits the refreshed `crates/crosswire-server/assets/ui/` in the same commit; `just ui-dist-check` must stay green.
- **UI dependency hygiene:** no new runtime deps without justification; exact-pinned versions; `ui/package-lock.json` committed; same-origin only (the CSP forbids external origins).
- **Untrusted content (§11.1):** every captured value (repeater `raw`/response bytes, held-item host/method/path, redirect URLs, scope host patterns, server error messages) renders as **text only** — CodeMirror or Svelte text interpolation, never `{@html}`/`innerHTML`/`insertAdjacentHTML`/`srcdoc`/`javascript:` href/`<svelte:element this>`. The CI lint enforces this over `ui/src` and **is tightened in Task 0** to also catch `setAttribute('srcdoc'|'on*', …)`, spread-attribute sinks, and dynamic `href`/`src`/`action` bindings (the Plan 5a carry-forward — relevant the moment any captured URL is rendered as a link).
- **Byte fidelity (§7.2):** the editable editor must round-trip every byte `0x00–0xFF`; paste-raw → send must send exactly the bytes shown; the Content-Length chip must never alter a message carrying `Transfer-Encoding` (show "skipped: TE present"); line-ending normalization is an assisted-mode toggle, off in raw mode.
- **Credentials (§11.2):** the UI adds no new unauthenticated routes and no new server authority; all calls go through the Plan 5a cookie-authenticated client (`credentials: 'same-origin'`, `Sec-Fetch-Site: same-origin`). PUT /scope needs the `admin` cap and send needs `send`; the bootstrap session carries them.
- **Commits:** Conventional Commits, **≤72-char subject**, a real why-body, the user's git identity (Kalash Poddar <kalashpoddar33@gmail.com>), and **no attribution trailers**. Verify `git log -1 | grep -c -i claude` is 0 after each commit.

## Review Focus

- **Paste-raw request carrying the full `0x00–0xFF` range → send:** the bytes sent (and echoed in the result) must equal the pasted bytes exactly; raw mode applies no assisted edits — Task 2 paste-raw byte-round-trip gate.
- **Content-Length chip on a `Transfer-Encoding` message (assisted mode):** the chip shows "skipped: TE present" and the message body is untouched; it never produces a double-framed request — Task 0/2 test.
- **Intercept held item whose body exceeded the hold limit (`editable:false`):** the editor opens read-only with a "not editable" indicator; forward sends the original bytes; editing is impossible, never a silent truncation — Task 3 test.
- **A scope PUT that does not compile (`422 scope_invalid`):** the editor surfaces the error and the previously-applied scope stays live and shown; no blank editor, no crash, no half-applied state — Task 4 test.
- **An XSS corpus in a repeater buffer, an intercept held item's host/path, or an intercept editor:** nothing executes — no dialog, no navigation, no off-origin request — Task 6 gate (the §13 corpus extended from history to repeater + intercept).

---

### Task 0: Editable `ByteEditor` + assisted-mode chips; tighten the untrusted-content lint

**Files:**
- Modify: `ui/src/lib/editor/ByteEditor.svelte` (enable editing when `readonly={false}`; expose edited bytes; the assisted/raw chip surface; paste-raw/copy-raw)
- Create: `ui/src/lib/editor/chips.ts` (the assisted-toggle + change-list chip model), `ui/src/lib/editor/assisted.ts` (client-side detection: is `Transfer-Encoding` present → CL chip "skipped: TE present"; the chip state is display only — the server applies edits on send)
- Modify: `ui/eslint.config.js` (extend the §11.1 bans — `setAttribute('srcdoc'|'on*')`, spread attributes, dynamic `href`/`src`/`action`)
- Test: `ui/src/lib/editor/assisted.test.ts`, `ui/src/lib/editor/editor-edit.spec.ts` (Playwright, via the Task-6-of-5a fixture extended to an editable mount)

**Interfaces:**
- Consumes: `ByteEditor` read-only behavior + `encoding.ts` (Plan 5a).
- Produces: `ByteEditor` with a working `readonly={false}` mode — a bound editable `bytes: Uint8Array` that round-trips every byte, the non-Latin-1 paste chip now active, paste-raw (replace the whole buffer from pasted text via `encodePastedText`) and copy-raw (copy the buffer bytes as Latin-1 text); `assisted.ts` exposing `assistedChips(raw: Uint8Array, toggles): Chip[]` that reports the Content-Length and line-ending chips and emits the "skipped: TE present" chip when the request carries `Transfer-Encoding`; `renderChanges(changes: ChangeDto[]): Chip[]` turning a `SendOutcomeDto.changes` list into result chips. The lint change adds the new sink rules as errors over `ui/src`.

- [ ] **Step 1: Write the failing tests** — `assisted.test.ts`: a request head with `Transfer-Encoding: chunked` yields a Content-Length chip whose state is "skipped: TE present"; a head with `Content-Length` and the toggle on yields an "update" chip; line-ending toggle reflects CRLF normalization intent; `renderChanges` maps an op/reason/start/end list to labeled chips. `editor-edit.spec.ts`: an editable `ByteEditor` seeded with all 256 byte values lets you edit one byte and reads the full buffer back byte-exact (the §7.2 editable golden).
- [ ] **Step 2: Run** `npm --prefix ui test` and `npm --prefix ui run e2e` → FAIL.
- [ ] **Step 3: Implement** the editable mode (CodeMirror editable state, same Latin-1 doc model and `lineSeparator.of('\n')` guard as read-only), `chips.ts`/`assisted.ts`, paste-raw/copy-raw; extend `eslint.config.js`. Prove the new lint rules fire on a temporary fixture, then remove it.
- [ ] **Step 4: Run** both suites → PASS; `npm --prefix ui run lint` clean; `just ui-build && just ui-dist-check` green. `ByteEditor.svelte` is already imported by the history detail, so the committed bundle **will** change — commit the refreshed `crates/crosswire-server/assets/ui/` with the source.
- [ ] **Step 5: Commit** `feat(ui): editable byte editor with assisted chips; harden lint`.

---

### Task 1: Repeater store — tabs, drafts autosave, send

**Files:**
- Create: `ui/src/lib/stores/repeater.svelte.ts`, `ui/src/lib/api/repeater.ts` (typed wrappers over the repeater routes + the `SendBody`/`SendOutcomeDto`/`DraftDto` types sourced from `repeater.rs`)
- Test: `ui/src/lib/stores/repeater.test.ts`

**Interfaces:**
- Consumes: `apiGet`/`apiSend` (Plan 5a).
- Produces: a `repeater` store (`$state`) with `tabs: RepeaterTab[]` (each tab holds the connection settings — host/port/tls/sni/verify/reuse/follow — the `mode: 'assisted'|'raw'`, the request `raw` bytes, and the last `SendOutcomeDto`), `newTab(seed?)`, `closeTab(id)`, `activeTab`, `selectTab(id)`, `send(tabId)` (builds `SendBody`, calls `POST /repeater/send`, stores the outcome), and **debounced draft autosave** (`PUT /repeater/tabs/{id}` with the tab's serialized `json`, ~500 ms after the last edit) plus load-from-`GET /repeater/tabs` at open. `sendFromExchange(exchangeId)` seeds a new tab from a history exchange (the Plan 5a history context action).

- [ ] **Step 1: Write the failing tests** — `repeater.test.ts` (mock `fetch`/client): `send` posts a correct `SendBody` (host/port/raw/mode/follow/reuse) and stores the returned `SendOutcomeDto`; a `422`/scope error surfaces without losing the draft; editing a tab schedules a single debounced `PUT /repeater/tabs/{id}` (fake timers); `newTab`/`closeTab`/`selectTab` manage tab state; `GET /repeater/tabs` at open restores drafts.
- [ ] **Step 2: Run** `npm --prefix ui test` → FAIL.
- [ ] **Step 3: Implement** the store + `repeater.ts` wrappers/types.
- [ ] **Step 4: Run** → PASS; `just ui-dist-check` green (store not yet imported by a view → bundle likely unchanged).
- [ ] **Step 5: Commit** `feat(ui): repeater store with tabs, debounced draft autosave, send`.

---

### Task 2: Repeater view + paste-raw byte-round-trip gate

**Files:**
- Create: `ui/src/routes/Repeater.svelte`, `ui/src/lib/components/RepeaterTab.svelte`, `ui/src/lib/components/ConnSettings.svelte`, `ui/src/lib/components/ResultHeader.svelte`
- Modify: `ui/src/lib/router.svelte.ts` (route `/repeater` → Repeater), `ui/src/lib/components/Shell.svelte` (nav entry)
- Test: `ui/src/routes/repeater.spec.ts` (Playwright), incl. the paste-raw round-trip gate

**Interfaces:**
- Consumes: `repeater` store (Task 1), editable `ByteEditor` + chips (Task 0), `ResultHeader` reads `SendOutcomeDto`.
- Produces: the repeater pane — a tab strip; per-tab `ConnSettings` (host, port, TLS, SNI distinct from the Host header, cert `verify`, fresh-vs-reused, follow off/in-scope/always); the request `ByteEditor` editable with the assisted/raw toggle showing the assisted chips, and raw mode showing the request bytes untouched; `ResultHeader` showing TTFB/total ms, the `scope_state`, the upstream-cert-override badge (`cert_override`), the applied `changes` as chips and `skipped` reasons, and the response via a read-only `ByteEditor`; paste-raw and copy-raw as first-class buttons/chords.

- [ ] **Step 1: Write the failing test** — `repeater.spec.ts` (mock the API with `page.route`): opening `/repeater` shows a tab; setting host/port and pasting a raw request then Send posts a `SendBody` and renders the `ResultHeader` (ttfb/total/scope_state) and response bytes. **Paste-raw gate:** paste a request whose body contains the full `0x00–0xFF` range in **raw** mode, Send, and assert the `raw` field the client posted equals the pasted bytes exactly (byte round-trip through the editable editor + raw path), and the rendered response bytes match the mocked response.
- [ ] **Step 2: Run** `npm --prefix ui run e2e` → FAIL.
- [ ] **Step 3: Implement** the view + components + the route/nav wiring.
- [ ] **Step 4: Run** → PASS; `npm --prefix ui run lint` clean; `just ui-build && just ui-dist-check` green (commit the refreshed bundle).
- [ ] **Step 5: Commit** `feat(ui): repeater view with assisted/raw modes and paste-raw`.

---

### Task 3: Request intercept — store, held list, actions, rules editor

**Files:**
- Create: `ui/src/lib/stores/intercept.svelte.ts`, `ui/src/lib/api/intercept.ts` (typed wrappers + `InterceptStatus`/`HeldDto`/`RulesDto`/`ForwardBody` types from `intercept.rs`), `ui/src/routes/Intercept.svelte`, `ui/src/lib/components/HeldList.svelte`, `ui/src/lib/components/RulesEditor.svelte`
- Modify: `ui/src/lib/components/StatusBar.svelte` (the off-by-default intercept toggle lives here), `ui/src/lib/router.svelte.ts` (route `/intercept`), `ui/src/lib/components/Shell.svelte` (nav)
- Test: `ui/src/lib/stores/intercept.test.ts`, `ui/src/routes/intercept.spec.ts` (Playwright)

**Interfaces:**
- Consumes: `apiGet`/`apiSend` + `apiGetBytes` (for a held item's current head/body bytes), the `events` store (the `{type:'intercept',held}` frame refreshes the list), the editable `ByteEditor` (Task 0).
- Produces: an `intercept` store with `enabled`, `held: HeldDto[]` (oldest focused), `rules: RulesDto`, `setEnabled(on)` (`PUT /intercept/enabled`), `forward(id, editedHead?, editedBody?)` (`POST /intercept/held/{id}/forward` with base64 head/body when edited), `drop(id)`, `forwardAll()`, `putRules(rules)`, and an `events`-driven refresh + `onLagged` refetch. `Intercept.svelte` shows the held list and, for the focused item, the intercept editor (the Task 0 editable `ByteEditor`, read-only with a "not editable" badge when `editable:false`); `RulesEditor.svelte` edits `skip_extensions`/`skip_out_of_scope`/`overflow`/`on_timeout`. `send-to-repeater` from a held item seeds a repeater tab (Task 1).

- [ ] **Step 1: Write the failing tests** — `intercept.test.ts` (mock client): `setEnabled` posts `{enabled}`; `forward` base64-encodes an edited head/body into `ForwardBody`; `drop`/`forwardAll` hit the right routes; an `{type:'intercept',held}` event triggers a `GET /intercept` refresh; a held item with `editable:false` is exposed as non-editable. `intercept.spec.ts`: toggling intercept on (from the status bar) shows the held list from a mocked `GET /intercept`; editing the focused item and Forward posts a `ForwardBody`; a non-editable item renders read-only with the badge and forwards the original bytes; the rules editor round-trips a `RulesDto` PUT.
- [ ] **Step 2: Run** both → FAIL.
- [ ] **Step 3: Implement** the store, API wrappers, view, held list, rules editor; wire the status-bar toggle.
- [ ] **Step 4: Run** → PASS; lint clean; `just ui-build && just ui-dist-check` green (commit the bundle).
- [ ] **Step 5: Commit** `feat(ui): request intercept with held list, edit-forward and rules`.

---

### Task 4: Scope editor — rules, import, enforce-strict

**Files:**
- Create: `ui/src/lib/stores/scope.svelte.ts`, `ui/src/lib/api/scope.ts` (typed wrappers + a `Scope`/`ScopeRule` type sourced from `crosswire-scope`), `ui/src/routes/Scope.svelte`, `ui/src/lib/components/ScopeRules.svelte`, `ui/src/lib/components/ScopeImport.svelte`
- Modify: `ui/src/lib/router.svelte.ts` (route `/scope`), `ui/src/lib/components/Shell.svelte` (nav); `ui/src/lib/components/StatusBar.svelte` (reflect scope state), `ui/src/routes/History.svelte` (activate the "add host to scope" context action stubbed in 5a)
- Test: `ui/src/lib/stores/scope.test.ts`, `ui/src/routes/scope.spec.ts` (Playwright)

**Interfaces:**
- Consumes: `apiGet`/`apiSend` (Plan 5a).
- Produces: a `scope` store with the current `Scope` document, `load()` (`GET /scope`), `save(scope)` (`PUT /scope` — needs `admin`; on `422 scope_invalid` keep the live scope and surface the compile error), `importList(text)` (`POST /scope/import` → a parsed `Scope` previewed before the user saves it), and `setEnforceStrict(on)` (`PUT /settings/enforce_scope_strictly`). `Scope.svelte` edits the ordered include/exclude rules (host pattern + optional ports/scheme/path), the import pane pastes a program list and previews the parse, and the enforce-strict toggle; scope state is reflected in the status bar and the history `scope_state` column.

- [ ] **Step 1: Write the failing tests** — `scope.test.ts` (mock client): `save` PUTs the scope JSON; a `422` response is surfaced as an error and the store keeps the previously-loaded scope (no half-applied state); `importList` posts `{text}` and exposes the parsed scope as a preview without saving; `setEnforceStrict` posts `{enabled}`. `scope.spec.ts`: editing a rule and Save round-trips; pasting a program list previews parsed rules then Save applies them; an invalid scope shows the error and leaves the editor populated with the prior rules.
- [ ] **Step 2: Run** both → FAIL.
- [ ] **Step 3: Implement** the store, wrappers, view, rules + import components; wire status-bar + the history context action.
- [ ] **Step 4: Run** → PASS; lint clean; `just ui-build && just ui-dist-check` green (commit the bundle).
- [ ] **Step 5: Commit** `feat(ui): scope editor with ordered rules, import and strict mode`.

---

### Task 5: Keymap handlers + keyboard operability + the mouse-free-core-loop gate

**Files:**
- Modify: `ui/src/lib/components/Shell.svelte` (register the keymap action handlers for the mounted views), `ui/src/routes/History.svelte` (keyboard-operable rows — the Plan 5a carry-forward: rows openable/selectable via keyboard with a `role`/key handler, not mouse-only), `ui/src/routes/Repeater.svelte`, `ui/src/routes/Intercept.svelte` (register `send`, `forward`, `drop`, `toggle-intercept`, `tab-next/prev`, `send-to-repeater`, `decode-selection` where they apply)
- Test: `ui/src/routes/mouse-free.spec.ts` (Playwright, the §7.6 gate)

**Interfaces:**
- Consumes: the `keymap` (Plan 5a `register`/`remap`/dispatch), the repeater/intercept/history stores.
- Produces: the Burp chords wired to real actions across the app — `send-to-repeater` (from history/intercept), `send` (repeater), `forward`/`drop`/`toggle-intercept` (intercept), `tab-next`/`tab-prev` (repeater tabs), `decode-selection` (a minimal inline decode of the editor selection, or a documented no-op stub if the decoder pane is beta-deferred — the chord must still be registered so the dispatch is exercised). Focus handling: a chord fires its action even from inside the editor where that is the Burp behavior (e.g. `send` from the repeater editor); registering a handler is what makes the 5a keymap `preventDefault` that chord, so only register chords that should be global.

- [ ] **Step 1: Write the failing test** — `mouse-free.spec.ts` (against mocked API or, if feasible, reuse the real-serve harness): drive the whole core loop from the keyboard only — select a history exchange, `send-to-repeater`, edit a byte in the repeater editor, `send`, toggle intercept, and `forward`/`drop` a held item — with no mouse clicks, asserting each step's visible result.
- [ ] **Step 2: Run** `npm --prefix ui run e2e` → FAIL (handlers not registered / rows not keyboard-operable).
- [ ] **Step 3: Implement** the handler registrations and keyboard operability.
- [ ] **Step 4: Run** → PASS; lint clean; `just ui-build && just ui-dist-check` green (commit the bundle).
- [ ] **Step 5: Commit** `test(ui): mouse-free core loop drives the Burp keymap end to end`.

---

### Task 6: XSS corpus extended to repeater + intercept

**Files:**
- Modify: `ui/e2e/fixtures/xss-corpus.ts` (reuse the 5a corpus), `ui/e2e/xss-corpus.spec.ts` (add repeater + intercept cases) — or create `ui/e2e/xss-repeater-intercept.spec.ts` reusing the fixture
- Test: the spec is the test

**Interfaces:**
- Consumes: the repeater view (Task 2), the intercept view (Task 3), the editable editor (Task 0), rendering seeded corpus data.
- Produces: the §13 XSS gate extended from history (5a) to repeater and intercept — seed the corpus into a repeater response and a loaded draft, and into an intercept held item's host/method/path and its held bytes, open each surface, and assert nothing executes (no dialog, no navigation, no off-origin request), reusing the 5a execution-detection listeners. Extend the self-test to confirm the detectors still fire.

- [ ] **Step 1: Write the failing test** — add repeater + intercept corpus cases to the gate; first confirm (lint-safely, via the 5a controlled-injection self-test) that the detectors catch execution on these surfaces, then assert the real surfaces render the corpus inertly.
- [ ] **Step 2: Run** `npm --prefix ui run e2e` → the new cases PASS (nothing executes), self-test confirms the listeners work.
- [ ] **Step 3: Implement/confirm** safe rendering (should already hold from Tasks 0/2/3 — text/CodeMirror only); fix any leak the gate finds.
- [ ] **Step 4: Run** → PASS; `just ui-lint` still clean (no `{@html}`); `just ui-dist-check` green.
- [ ] **Step 5: Commit** `test(ui): XSS corpus extended to repeater and intercept surfaces`.

---
