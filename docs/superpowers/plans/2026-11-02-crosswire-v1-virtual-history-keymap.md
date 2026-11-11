# V1 Virtual History and Scoped Keymap Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the last REPORT-ONLY §13 performance row by virtualising the History list (bounded DOM, auto-loading keyset pages, no frame > 32 ms, filter < 300 ms, asserted in CI at 100k), and give the keymap scopes so the viewer's Find is a real, remappable action that shares `Ctrl+F` with Intercept's Forward by design, with per-scope conflicts, grouped Settings and live hints.

**Architecture:** A pure `virtual-window.ts` module does all window, reveal, anchor and auto-load math; History keeps its native `<table>` with two spacer rows whose heights are CSSOM custom properties, renders `visibleRows.slice(start, end)`, and handles keys through one delegated list handler. The history store gains an in-flight guard, dedupe, a 50,000-row cap on paging and prepends, a `generation` counter, an id index and per-frame event coalescing. The keymap gains a `scope` per action, element-bound handlers, a DOM scope chain (`data-keyscope`) with a router fallback, per-scope conflict checks and a reactive hint module.

**Tech Stack:** Svelte 5 runes, TypeScript, Vitest (jsdom), Playwright. No new dependency, no Rust change.

**Spec:** `docs/superpowers/specs/2026-11-02-crosswire-v1-virtual-history-keymap-design.md` (section numbers below refer to it).

**Branch:** `feat/v1-virtual-history-keymap` from `main` once U2 is merged (if U2 is still open, from the U2 tip and rebase after it merges). Copy the spec and this plan into `docs/superpowers/{specs,plans}/` as the first commit: `docs(superpowers): V1 virtual history and scoped keymap spec and plan`.

## Global Constraints
- §11.1: rows and hints render text only; no `{@html}`, `innerHTML`, `srcdoc`, dynamic `href`/`src`, `<svelte:element>`; ESLint config unchanged; no new `eslint-disable`.
- CSP: no `style=` attribute, no `style:` directive. The only style writes are `listEl.style.setProperty('--vt-before'|'--vt-after', `${n}px`)` with `n` computed from indices and the measured row height.
- `ui/package.json` and `ui/package-lock.json` unchanged. No Rust, API, OpenAPI or store change.
- Every existing `data-testid` kept; History's are added to `ui/src/lib/testids.test.ts`.
- Bundle rebuilt (`just ui-build`) and committed in the integration task only; `just ui-dist-check` clean; budget +6 KB JS / +1 KB CSS raw over the U2 bundle.
- Conventional Commits, subject ≤ 72 chars, body says why; NO `Co-Authored-By`/`Claude-Session` or any AI attribution trailer.
- Cargo commands run sequentially, never piped through `tail`; the full workspace `cargo test` runs once, in T5, in the background.
- UI gate per task (in `ui/`): `npm run lint && npm run check && npm run test`, then `npm run e2e` (vite-preview suite). Any existing spec edited must say why in the commit body.

## Review Focus
1. Scroll to the middle of 5,000 rows, focus a row, wheel it out of view, press `j`: focus is on the next row, which is revealed; focus was never on `<body>` (T1 e2e).
2. 300 `received` events while scrolled to the middle: the anchored row stays at the same viewport offset (±1 px), one window pass per frame (T0 unit + T1 e2e).
3. Rows at the 50,000 cap plus live prepends: Load more disabled with `Limit reached`, no duplicate keys, no hole (T0 unit + T1 e2e).
4. Ctrl+F in a History viewer on the Raw tab opens find; on the Headers tab nothing in the app fires and the browser keeps the key; on the Intercept route Ctrl+F forwards (T2 unit + T3 e2e).
5. The U2 stored overrides `{"forward":"ctrl+alt+f","toggle-intercept":"ctrl+f"}` load verbatim and `settings.spec.ts` "Ctrl+F inside a viewer…" passes unchanged (T2 unit + T3 e2e).
6. `just ui-perf` at 100k asserts and passes: `max_frame_ms ≤ 32`, `filter_ms < 300`, `dom_rows ≤ 2 × visible + 8`, `pages_loaded ≥ 1`, `loaded_rows ≥ 1000` (T4/T5).

## Parallelism
```
T0 (store + window math) ──> T1 (History virtual list) ──> T4 (perf gate) ──┐
T2 (keymap core) ──────────> T3 (keymap consumers, no History.svelte) ──────┴─> T5 (integration)
```
T0→T1→T4 and T2→T3 run in two worktrees in parallel; they share no file. T3 must not edit `ui/src/routes/History.svelte` (T1 rewrites it); the History keymap wiring lands in T5.

---

### Task 0: virtual-window math and history store (sonnet)
**Files:** Create `ui/src/lib/components/virtual-window.ts`, `ui/src/lib/components/virtual-window.test.ts`; Modify `ui/src/lib/stores/history.svelte.ts`, `ui/src/lib/stores/history.test.ts`.

**Interfaces → Produces:**
```ts
// virtual-window.ts
export const OVERSCAN = 8;
export const AUTOLOAD_MARGIN = 200;
export interface Win { start: number; end: number; before: number; after: number }
export function windowFor(scrollTop: number, viewportH: number, rowH: number, headH: number, total: number, overscan?: number): Win;
export function revealTop(index: number, scrollTop: number, viewportH: number, rowH: number, headH: number, footH: number): number | null;
export function anchorAt(scrollTop: number, rowH: number, headH: number, ids: readonly string[]): { id: string; offset: number } | null;
export function restoreAnchor(a: { id: string; offset: number }, indexOf: (id: string) => number, rowH: number, headH: number): number | null;
export function shouldAutoLoad(win: Win, total: number, s: { hasMore: boolean; loading: boolean; searching: boolean; capped: boolean }): boolean;

// history.svelte.ts (additions to createHistory's options and return value)
interface HistoryOptions { /* … */ maxRows?: number /* default 50_000 */; schedule?: (fn: () => void) => void }
// returned: get capped(): boolean; get generation(): number; indexOf(id: string): number; byId(id: string): Row | null;
```

- [ ] **Step 1: failing tests.** `virtual-window.test.ts`: `windowFor` at top (`start 0`), middle, bottom (`end === total`), `total` smaller than the viewport, `total` shrinking below `start` (clamped, `after ≥ 0`), `before + (end-start)*rowH + after === total*rowH`; rendered count ≤ `visible + 2*OVERSCAN`; `revealTop` returns `null` for a fully visible row, the top-aligned `scrollTop` (minus `headH`) for a row above, the bottom-aligned one (plus `footH`) for a row below; `anchorAt`/`restoreAnchor` round-trip after 3 ids are prepended (new `scrollTop` = old + 3·rowH), `null` when the anchor id is gone; `shouldAutoLoad` truth table (false while loading, searching, capped, `!hasMore`, or far from the end). `history.test.ts`: two concurrent `loadMore()` calls issue one GET; a page repeating a loaded id appends it once; with `maxRows: 4, pageSize: 2`, paging stops at 4 (`capped` true, `loadMore` issues no GET) and a prepend at the cap trims the tail and keeps `capped`; `generation` bumps on `applyFilter`/`reload` and not on `loadMore`/events; 5 `received` events with a manual scheduler produce one `rows` change when flushed (and none before); `indexOf`/`byId` follow prepends and appends. Update the existing `maxRows: 2` test to inject `schedule: (fn) => fn()`. Run `npm run test` → the new tests FAIL.
- [ ] **Step 2: implement** spec §3.1.4, §3.1.6, §3.1.7, §3.1.9 (store half): `rows` as `$state.raw`; `loading` guard in `loadMore`; dedupe on append via the id index; cap on append (`capped` when `rows.length >= maxRows`) and on prepend (trim tail); `generation`; event queue flushed by `schedule` (default `requestAnimationFrame`, `setTimeout(fn, 16)` when rAF is absent); `indexOf`/`byId` from a `Map` rebuilt when `rows` changes. `patchRow`/`bulkTag` behaviour unchanged.
- [ ] **Step 3: gate** `npm run lint && npm run check && npm run test` green; `npm run e2e` green (History still non-virtual here; nothing visible changes except the cap default).
- [ ] **Commit** `feat(ui): window math and a bounded, coalescing history store`. Body: why the in-flight guard (duplicate-key crash), why the cap bounds paging too (the tail hole), why `$state.raw`.

### Task 1: virtualised History list (opus)
**Depends on:** T0. **Files:** Modify `ui/src/routes/History.svelte`, `ui/src/lib/components/history-view.ts` (+ test: `pageStep(visible)`, key → delta table incl. `Home`/`End`/`PageUp`/`PageDown`), `ui/src/lib/testids.test.ts` (History snapshot); Create `ui/e2e/history-virtual.spec.ts`. Do not touch the keymap or the `<svelte:window onkeydown>` `/` handler (T5).

**Interfaces → Produces (DOM contract, spec §3.1.1, §3.1.11):**
- `tbody` = `tr.pad[data-testid="history-pad-top"][aria-hidden="true"]` + rows + `tr.pad[data-testid="history-pad-bottom"]`.
- Data rows: `tr.row[data-testid="history-row"][data-id][aria-rowindex]`, `tabindex` 0 on the roving row only; `listEl` `tabindex` 0 only while the roving row is not rendered.
- `table[aria-rowcount]` = `-1` while `hasMore && !capped`, else `visibleRows.length + 1`; header `tr[aria-rowindex="1"]`.
- `load-more` labels `Load more` / `Loading…` / `End` / `Limit reached`; `history-cap-note` at the cap.
- CSS custom properties on `listEl`: `--vt-before`, `--vt-after` (px integers or floats); `--history-row-h: 26px` on `.list`.

- [ ] **Step 1: failing e2e** `history-virtual.spec.ts` (mocked API like `history-nav.spec.ts`; a generator serves 5,000 rows in 100-row pages with `next_cursor` = the last id, and a second fixture serves 50,100 rows). Helpers: `scrollListTo(page, px)` sets `history-list.scrollTop` and waits two rAFs; `rowH(page)` reads the first row's height. Tests: (a) DOM bound — after load and after scrolling to 25% / 50% / bottom, `history-row` count ≤ `2*ceil(clientHeight/rowH) + 8`, pads present, sticky header still at the list's top; (b) `aria-rowcount` `-1` with more pages, `n+1` at `End`; first rendered row's `aria-rowindex` = its `#` cell + 1; (c) focus the last rendered row, press `j` → the next row is focused and inside the list's visible box; `End` → last loaded row focused; `Home` → first; (d) focus a row, wheel 3 viewports down → `document.activeElement` is `history-list`, never `body`; wheel back → the row is focused again; `j` from the list acts on that row; (e) check a row, scroll it out and back → still checked; footer still says `1 selected`; select-all → `N loaded · N selected` with N = all loaded rows; (f) scroll to the middle, inject 300 `received` events through the mocked WebSocket (reuse the events mock used by `history.spec.ts`) → the first fully visible row id and its offset are unchanged (±1 px); at `scrollTop` 0 the new rows are at the top; (g) scroll to the bottom → an `after=` request is made without clicking; `load-more` via `Tab`+`Enter` also appends; (h) 50,100-row fixture: scroll to the bottom repeatedly → `load-more` reads `Limit reached`, `history-cap-note` visible, rows = 50,000; (i) every rendered row's height = `rowH` ± 0.5 px with a 300-char host and 10 tags; (j) `pageerror` list empty throughout. Run `npm run e2e -- history-virtual` → FAIL.
- [ ] **Step 2: failing unit tests** for `history-view.ts` additions and the History testid snapshot (`history-view`, `history-list`, `history-row`, `select-all`, `load-more`, `column-picker`, `column-menu`, `history-count`, `history-search`, `history-empty`, `history-detail`, `uncommitted-marker`, `filter-*`, `bulk-*`) → FAIL for the new helpers only.
- [ ] **Step 3: implement** spec §3.1: fixed table layout and `<col>` widths (§3.1.1 table; tune until `discoverability.spec.ts` "fits" passes unchanged at both viewports), measure `rowH`/`headH`/`footH` once and on `ResizeObserver` (§3.1.2), single-line cells (§3.1.3), the window and pads (§3.1.4), the passive scroll listener + single rAF (§3.1.5), auto-load + Load more + cap note (§3.1.6–7), delegated keys, reveal, evacuation and the list tab stop (§3.1.8), anchor restore in `$effect.pre` keyed on `visibleRows` with `generation` reset (§3.1.9), search/sort reset to top, default sort uses store order, selection not pruned on window exit (§3.1.10), ARIA (§3.1.11). Replace `rowById`/`rovingId`/`moveFocus` lookups with `history.indexOf`/`byId` and a per-`visibleRows` index map. `closeMenu` refocuses only a connected row.
- [ ] **Step 4: gate** `npm run lint && npm run check && npm run test`; `npm run e2e` — the whole suite, including `history-nav`, `discoverability`, `history`, `tags-notes`, `mouse-free`, `xss-corpus` unchanged (spec §3.1.12). If an existing spec must change, change only waits/scrolling, and name the reason in the commit body.
- [ ] **Commit** `feat(ui): virtualise the history list with bounded DOM rows`. Body: why spacer rows in a table (selectors, sticky header, column alignment), why fixed layout, why auto-load fires only from scroll.

### Task 2: scoped keymap core (opus, worktree; parallel with T0/T1)
**Files:** Modify `ui/src/lib/keymap.ts`, `ui/src/lib/keymap.test.ts`; Create `ui/src/lib/keymap-hints.svelte.ts`, `ui/src/lib/keymap-hints.test.ts`.

**Interfaces → Produces:**
```ts
export type Scope = 'global' | 'history' | 'repeater' | 'intercept' | 'viewer';
export const SCOPES: readonly Scope[];                 // display order: global, history, repeater, intercept, viewer
export const SCOPE_LABELS: Readonly<Record<Scope, string>>;
export const PARENTS: Readonly<Record<Scope, readonly Scope[]>>;
export function coActive(a: Scope, b: Scope): boolean;
export const ACTIONS: readonly [...U2 actions, 'focus-search', 'find'];
export const ACTION_SCOPES: Readonly<Record<Action, Scope>>;
export type Handler = (e: KeyboardEvent) => void | boolean;   // false = decline
export interface Binding { action: Action; chord: string | null; label: string; scope: Scope }
export function routeScopeFor(path: string): Scope | null;     // '/'→history, '/repeater'→repeater, '/intercept'→intercept
export function scopeChain(target: EventTarget | null, routeScope: Scope | null): Scope[];
// createKeymap(...) return value changes:
//   register(action, handler, opts?: { el?: Element }): () => void
//   dispatch(e: KeyboardEvent, routeScope?: Scope | null): void
//   bindings(): Binding[]            // every action, unbound ones with chord null
//   shadows(action: Action): Action[]
//   validateChord(chord, action)     // per co-active scope; message names holder + scope label
// keymap-hints.svelte.ts
export function chordLabel(action: Action): string | undefined;   // reactive
export function findHint(): string | undefined;                   // "<Chord> in the viewer"
```
`DEFAULT_BINDINGS` gains `'focus-search': '/'` and `find: 'ctrl+f'`; `ACTION_LABELS` gains `'Focus the History search'` and `'Find in the message'`. `VIEWER_FIND_CHORD` and the old `findHint` are removed from `keymap.ts`.

- [ ] **Step 1: failing tests** in `keymap.test.ts`: defaults table incl. the two new actions; every action has a scope; **no co-active clash among defaults** (enumerated pairwise); `coActive` table (viewer↔history true, viewer↔intercept false, history↔repeater false, global↔any true); `scopeChain` from a target inside `[data-keyscope=viewer]` inside `[data-keyscope=history]` → `['viewer','history','global']`, from `document.body` with route `intercept` → `['intercept','global']`; dispatch: with Find (viewer, element-bound to viewer A) and Forward (intercept) both on `ctrl+f`, a key in viewer A calls Find only; a key outside any viewer with route `intercept` calls Forward; two viewers A and B bound — a key in B calls B's handler; a handler returning `false` → no `preventDefault`, no outer handler called; a plain `?` inside `[role=menu]` does nothing; `validateChord('ctrl+f','find')` ok (disjoint from Forward) while `validateChord('ctrl+r','copy-raw')` refuses naming "Send to Repeater" (Everywhere); `remap` steals only from co-active holders; `bindings()` lists a stolen action with `chord: null`; U2 storage fixtures: `{forward:'ctrl+alt+f','toggle-intercept':'ctrl+f'}` loads verbatim (both applied, `find` keeps `ctrl+f`, `shadows('find')` = `['toggle-intercept']`, dispatch inside a viewer → Find, outside → toggle); a same-scope U2 clash still resolves order-independently (existing test kept); `{"copy-raw":"/"}` loads and `focus-search` shadows it inside History. `keymap-hints.test.ts`: inside `$effect.root`, a `$derived(chordLabel('send'))` updates after `keymap.remap` and `reset`. Run → FAIL.
- [ ] **Step 2: implement** spec §3.2.1–3.2.7 and §3.2.8's hint module. Keep `chordFromEvent`, `normalizeChord`, `RESERVED`, `modifierless`, `recordChord` behaviour unchanged. `isEditable` stays; add the menu/dialog check for plain-key chords only.
- [ ] **Step 3: gate** `npm run lint && npm run check && npm run test`. (No consumer is migrated yet: the old `register(action, handler)` call shape still compiles; `dispatch(e)` without a route scope behaves as U2 for all U2 actions.) `npm run e2e` green.
- [ ] **Commit** `feat(ui): scoped keymap with per-scope conflicts and live hints`. Body: why innermost-scope-wins, why load keeps cross-scope U2 clashes as shadows.

### Task 3: keymap consumers (sonnet, worktree; after T2)
**Depends on:** T2. **Must not edit** `ui/src/routes/History.svelte`. **Files:** Modify `ui/src/lib/components/Shell.svelte` (dispatch with `routeScopeFor(router.path)`), `ui/src/routes/Repeater.svelte` and `ui/src/routes/Intercept.svelte` (`data-keyscope` on the root; Intercept hints via `keymap-hints`), `ui/src/lib/viewer/MessageViewer.svelte` (drop the local listener; `data-keyscope="viewer"`; register `find` bound to `root`, declining off textual tabs; hint via `findHint()`), `ui/src/lib/copy-actions.svelte.ts` (remove `chordLabel`, import from `keymap-hints`), `ui/src/lib/components/{TopBar,SelectionToolbar,RowContextMenu,ExchangeDetail,RepeaterTab,RepeaterResponse,ShortcutsDialog}.svelte`, `ui/src/lib/components/row-actions.ts`, `ui/src/routes/Settings.svelte` (groups by scope, `shortcut-group-<scope>`, "Not set", `shortcut-shadow-<action>` note); Create `ui/e2e/keymap-scopes.spec.ts`; Modify `ui/e2e/settings.spec.ts` only if a selector moved (none expected).

- [ ] **Step 1: failing e2e** `keymap-scopes.spec.ts` (mocks as in `settings.spec.ts`): (a) defaults — History detail Raw tab, focus the response viewer, `Control+KeyF` → `detail-findbar-input` focused; Intercept route with one held item, `Control+KeyF` → a forward POST is made; (b) Settings → Change `find` to `Ctrl+Shift+F` → accepted; `detail-find` title reads `Find in message (Ctrl+Shift+F in the viewer)` on return to History; (c) Settings → Change `copy-raw` (global) to `Ctrl+F` → refused, because a global chord is co-active with both Forward (intercept) and Find (viewer); the message names the first co-active holder in `ACTIONS` order: `Ctrl+F is already used by "Forward the held request (Intercept)" (Intercept).`; then Change `send` (repeater) to `Ctrl+D` → accepted (Drop is intercept, disjoint); (d) TopBar switch title updates while Settings is open after remapping Toggle intercept (existing test kept) and the selection-toolbar Send button `kbd` shows the new Send-to-Repeater chord on return to History; (e) Settings shows groups Everywhere, History, Repeater, Intercept, Message viewer in that order, each with its actions; (f) with U2 storage `{forward:'ctrl+alt+f','toggle-intercept':'ctrl+f'}` the Find row shows `shortcut-shadow-find`; (g) `?` sheet shows the scope groups and the fixed "History list" group. Run → FAIL.
- [ ] **Step 2: implement** spec §3.2.3 (Shell), §3.2.4 (MessageViewer), §3.2.8 (all hints reactive; ShortcutsDialog rows `$derived`), Settings grouping and unset/shadow display (§3.2.6).
- [ ] **Step 3: gate** `npm run lint && npm run check && npm run test`; `npm run e2e` — whole suite; `settings.spec.ts` "Ctrl+F inside a viewer opens find and never reaches the global keymap" must pass **unchanged** (it is the U2 migration pin), as must `mouse-free.spec.ts` step 6 (Ctrl+F forwards in Intercept).
- [ ] **Commit** `feat(ui): find as a viewer action, grouped shortcuts, live hints`. Body: why the viewer listener went (one dispatch table), the macOS Cmd+F change (spec §3.2.4).

### Task 4: asserting perf gate (sonnet; after T1)
**Depends on:** T1. **Files:** Modify `ui/e2e/perf-scroll.spec.ts`, `scripts/ui-perf.sh`, `ui/playwright.perf-scroll.config.ts`, `.github/workflows/ci.yml`, `justfile` (comments on `ui-perf` and `perf`), `bench/README.md` (§13 ledger row and the UI section's status; numbers filled in T5).

- [ ] **Step 1:** rewrite the driver per spec §3.3.1: wheel only (remove the Load-more click loop), count `after=` list requests with `page.on('request')`, read `visible_rows` and `loaded_rows` after the measured window, import `OVERSCAN` from `../src/lib/components/virtual-window`, extend the SUMMARY line with `visible_rows=` and `loaded_rows=`. Assertions under `process.env.CROSSWIRE_UI_PERF_ASSERT === '1'`: `maxFrame ≤ 32`, `filterMs < 300`, `domRows ≤ 2*visible + OVERSCAN`, `pages ≥ 1`, `loadedRows ≥ 1000`; gross floors always. Update the header comment (no longer report-only at 100k).
- [ ] **Step 2:** `scripts/ui-perf.sh`: `ASSERT="${CROSSWIRE_UI_PERF_ASSERT:-}"; if [ -z "$ASSERT" ]; then if [ "$ROWS" -le 100000 ]; then ASSERT=1; else ASSERT=0; fi; fi; export CROSSWIRE_UI_PERF_ASSERT="$ASSERT"`; update the header comment. `playwright.perf-scroll.config.ts`: `retries: process.env.CI ? 1 : 0`.
- [ ] **Step 3:** `ci.yml`: after `just first-capture-ui`, add
  ```yaml
      # §13 UI gate, ASSERTED at 100k (max frame <= 32 ms, filter < 300 ms,
      # bounded DOM rows); the 1M run stays maintainer-only and report-only.
      - run: just ui-perf
        if: matrix.os == 'ubuntu-latest'
  ```
  `justfile`: `ui-perf` and `perf` comments say "asserting at ≤ 100k rows, report-only above". `bench/README.md`: §13 ledger row → "ASSERTED at 100k in CI; 1M report-only (maintainer)", the UI section's status paragraph rewritten (virtualised list, the assertion set), numbers table left with a `TBD (T5)` row.
- [ ] **Step 4: gate** `npm run lint && npm run check && npm run test` (the perf spec is type-checked by Playwright when it runs, not by `svelte-check`); then `just ui-perf` once locally at 100k (release build + seed; ~5 min) and paste the SUMMARY into the commit body.
- [ ] **Commit** `test(ui): assert the history scroll and filter gate at 100k`; `ci: run the asserting UI perf gate on ubuntu` (separate commit for the workflow).

### Task 5: integration (opus; main worktree)
- [ ] **Step 1: merge** T2/T3 onto the T0→T1→T4 line (cherry-pick in order; no file overlap except `ui/src/lib/components/*` hint call sites, which T1 does not touch). Resolve, run `npm run check`.
- [ ] **Step 2: History keymap wiring (failing test first).** Add to `keymap-scopes.spec.ts`: `/` on History focuses `history-search` and selects its text; `/` typed inside the search box or a filter input types a slash; `/` inside the column menu does nothing; remapping Focus-search to `Alt+/` in Settings makes `/` inert and `Alt+/` work on return. Run → FAIL. Then in `History.svelte`: `data-keyscope="history"` on the `history-view` root; register `focus-search` in `onMount` (handler: focus + select `history-search`, return `false` when the start panel shows and there is no search box); delete `onWindowKey` and `<svelte:window onkeydown>`; `send-to-repeater` registration unchanged; delegated list keys stay list-local (not keymap). Re-run → PASS.
- [ ] **Step 3: XSS corpus over virtualised rows** (spec §3.3.3): add to `xss-corpus.spec.ts` a test serving 2,000 rows (every 40th carries corpus payloads in method, host, path and tags; the rest benign), `watch(page, origin)` armed before `goto`; scroll `history-list` top→bottom→top in viewport steps waiting two rAFs each; assert: no violations; each payload row's `td.host`/`td.path` `textContent` equals the payload with newlines as the browser renders text (use the existing one-line oracle); `tbody` contains only `tr`/`td`/`input`/`span`/badge/chip elements (an allow-list query); every row height = `rowH` ± 0.5 px; `getComputedStyle(list).getPropertyValue('--vt-before')` matches `/^\d+(\.\d+)?px$/`. Must pass first time; a failure is a §11.1 bug to fix, not a test to relax.
- [ ] **Step 4: UI gates.** In `ui/`: `npm run lint`, `npm run check`, `npm run test`, `npm run e2e` (full suite, sequential). Then `just ui-build` and commit the bundle `build(ui): rebuild embedded bundle`; record the JS/CSS raw and gzip deltas against the U2 bundle in the commit body (budget +6 KB / +1 KB raw); `just ui-dist-check` clean.
- [ ] **Step 5: perf.** `just ui-perf` (100k, asserting) twice; then the maintainer run `CROSSWIRE_UI_PERF_ROWS=1000000 just ui-perf` (report-only; ~10 min seed + serve). Both must meet `max_frame ≤ 32 ms` and `filter < 300 ms`; fill the `bench/README.md` table (date, machine, rows, max/p99 frame, frames > 32 ms, filter, DOM rows/nodes, pages) replacing the `TBD (T5)` row; commit `docs(bench): record the virtualised history UI trace`. If 1M misses, stop and report to the maintainer with the trace; do not loosen thresholds.
- [ ] **Step 6: docs.** `CHANGELOG.md` `[Unreleased]`: *Changed* — History list virtualised (bounded DOM, auto-loading pages, Load more kept, 50,000-row client cap with a note); shortcuts grouped by scope, Find (`Ctrl+F`) is a remappable message-viewer shortcut sharing its chord with Intercept's Forward; `/` is the remappable "Focus the History search"; `Home`/`End`/`PageUp`/`PageDown` in the History list. *Fixed* — Load more could append a page twice; the live-row cap could leave a gap in paged history; shortcut hints that stayed stale after a remap; an action whose chord was taken over vanished from Settings. *Note* — on macOS `Cmd+F` inside the viewer now opens the browser's find; use `Ctrl+F`. Commit `docs(changelog): record virtual history and scoped shortcuts`.
- [ ] **Step 7: full CI gate** (sequential, never piped through `tail`): `just lint`, `just doc`, `cargo deny check` (or the CI action's equivalent), `just features`, `just ui-lint`, `just ui-test`, `just ui-dist-check`, `npm --prefix ui run e2e`, `just first-capture-ui` (on the dev Mac with `CROSSWIRE_BROWSER="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"`), `just ui-perf`; `just test` (40+ min) once, in the background, and wait for its exit code. No Rust changed, but the bundle is embedded by `crosswire-server`, so the Rust gate still runs.
- [ ] **Step 8: screenshots** of History (dark/light, 1440×900 and 1280×720) scrolled to the middle of 5,000 rows, and Settings shortcut groups; attach to the PR.
- [ ] **Step 9: whole-branch review** with superpowers:requesting-code-review against this plan's Review Focus; fix must-fix findings; record deferred ones in the PR body.
- [ ] **Step 10: PR** `feat: virtualised history and scoped keyboard shortcuts (V1)` to `main`; body lists the spec decisions, the perf table (100k CI, 100k and 1M maintainer), the bundle delta, the changed specs and why, and the three open questions (spec §7) with the defaults taken. No AI attribution lines.
