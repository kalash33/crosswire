# U1 "Proper App" UI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn the embedded web UI into a polished, discoverable professional tool (dark default, light option) without changing the backend, adding a dependency, or weakening §11.1.

**Architecture:** A token-based theme (`theme.css`, `data-theme`) and a small in-repo component vocabulary (`ui/src/lib/ui/`) replace browser defaults; the shell becomes a fixed-height grid (rail, top bar, scrolling main, pinned status bar); each view is rebuilt on those components with every action visible and its shortcut shown; History and Repeater get split panes with Headers/Raw/Hex tabs; Decoder and Settings become routes; a Start panel guides first traffic.

**Tech Stack:** Svelte 5, Vite, CodeMirror 6 (unchanged), inline SVG icons, CSS custom properties, Vitest, Playwright.

**Spec:** `docs/superpowers/specs/2026-10-31-crosswire-u1-proper-app-ui-design.md`

## Global Constraints
- No new npm dependency; `ui/package.json` and the lockfile unchanged.
- §11.1: text interpolation only; no `{@html}`, innerHTML, srcdoc, dynamic href/src; ESLint config unchanged, no new `eslint-disable`.
- CSP: no `style=` attributes, no runtime style injection; theme via `data-theme` on `<html>`; no external assets.
- All existing `data-testid`s listed in the spec inventory are preserved; selector changes only where the spec names them, in the same commit as the test update.
- Hex colours only in `ui/src/lib/theme.css` (pinned by a unit test).
- Gates per task: `npm --prefix ui run lint`, `npm --prefix ui run check` (svelte-check; add the script if absent, `svelte-check --tsconfig ./tsconfig.json`), `npm --prefix ui run test`. e2e and the bundle rebuild happen in T5 only (T0/T1 may run e2e since they run alone).
- Conventional Commits ≤72-char subjects, real bodies, NO AI attribution trailer.

## Review Focus
1. A captured header value containing `<img src=x onerror=…>` shown in the new Headers tab, the detail title, a toast or a context-menu label → rendered as text (XSS corpus extended in T5).
2. A 2 MB response opened in the Hex tab → the pane renders a bounded window, not the whole buffer as DOM (T2: cap rows and page).
3. Window narrower than 1280px → panes stack and nothing overflows the viewport; status bar stays visible (T0 test at 1024×700).
4. A WebSocket drop → status dot turns disconnected and recovers; no toast storm (T1: one toast per transition).
5. A mid-session 401 (server restarted) → Auth screen, not a frozen view (T1).

---

### Task 0: tokens, component vocabulary, shell layout fix
**Files:** Create `ui/src/lib/theme.css`, `ui/src/lib/theme.ts`, `ui/src/lib/ui/{Button,IconButton,Icon,Input,Select,Segmented,Tabs,Badge,Chip,Toolbar,EmptyState,Kbd,Menu,Splitter,Toast}.svelte`, `ui/src/lib/ui/icons.ts`, `ui/src/lib/ui/toasts.svelte.ts`, `ui/src/lib/format.ts` (`humanBytes`, `timeShort`). Modify `ui/src/app.css`, `ui/src/main.ts` (apply theme before mount), `ui/src/lib/components/Shell.svelte` (grid: `56px 1fr` × `44px 1fr 28px`, `height: 100dvh`, `overflow: hidden`, main scrolls), `ui/src/lib/components/StatusBar.svelte` (single line, pinned), `ui/package.json` scripts only (`check`). Tests: `ui/src/lib/theme.test.ts`, `ui/src/lib/ui/{menu,splitter,toasts}.test.ts`, `ui/src/lib/format.test.ts`, `ui/src/lib/no-hex.test.ts` (greps `ui/src/**/*.{svelte,ts,css}` except `theme.css` for `#[0-9a-fA-F]{3,8}\b` in CSS contexts; fails listing offenders), e2e `ui/e2e/layout.spec.ts` (at 1024×700 and 1440×900: `status-bar` bounding box fully inside viewport on every route; `document.scrollingElement.scrollHeight <= innerHeight`).
**Interfaces → Produces:** component props: `Button {variant?: 'primary'|'default'|'ghost'|'danger'; size?: 'sm'|'md'; icon?: IconName; kbd?: string; disabled?; onclick}`; `IconButton {icon, label (required, aria-label+title), kbd?}`; `Icon {name: IconName, size?=16}`; `Segmented {options: {value,label,icon?}[], value, onchange}`; `Tabs {tabs:{id,label,badge?}[], active, onselect}`; `Badge {kind:'method'|'status'|'state'|'scope'|'neutral', value}`; `Menu {items:{label,icon?,kbd?,danger?,onselect}[], open, anchor: HTMLElement|{x,y}, onclose}`; `Splitter {direction:'row'|'column', min=[200,240], storageKey}` with two snippets `a`/`b`; `toasts.push({kind:'info'|'success'|'error', text, ttlMs=4000})`; `theme.get()/set('dark'|'light')/toggle()`; `humanBytes(n): string` (`0 B`, `1.8 KB`, `2.3 MB`), `timeShort(iso): string` (`HH:MM:SS`).
- [ ] Step 1: write the failing unit tests above (theme persistence incl. storage throwing; menu arrow/Escape/Enter; splitter clamp to min and ratio persistence; toasts ttl and dedupe of identical consecutive text; format helpers; no-hex test initially FAILS on the existing ~30 hex values).
- [ ] Step 2: implement theme.css tokens (spec D1), theme.ts, components (spec D2), format.ts; convert every existing scoped style to tokens until no-hex passes; shell grid + status bar single line; `main.ts` applies theme.
- [ ] Step 3: `npm run lint && npm run check && npm run test` green; `npm run e2e` green incl. the new layout spec (this task runs alone, so e2e is allowed).
- [ ] Step 4: Commit `feat(ui): design tokens, component set and fixed app shell`.

### Task 1: shell chrome, Start view, toasts wiring, auth, routes
**Files:** Create `ui/src/lib/components/Rail.svelte`, `TopBar.svelte`, `ShortcutsDialog.svelte`, `StartPanel.svelte`; `ui/src/routes/Decoder.svelte` (hosts `DecoderPane` full-height; removes the fixed overlay), `ui/src/routes/Settings.svelte`. Modify `Shell.svelte` (rail + top bar + toasts outlet + `?` key), `ui/src/lib/router.svelte.ts` (add `/decoder`, `/settings`), `ui/src/App.svelte` (route branches; 401 → auth), `ui/src/lib/api/client.ts` (on 401 call `session.expire()`), `ui/src/lib/stores/session.svelte.ts` (`expire()`), `ui/src/routes/Auth.svelte` (restyle), `ui/src/routes/History.svelte` (show `StartPanel` when rows empty and no filter; hides on first `received`), `StatusBar.svelte` (WS dot from `events.connected`, proxy address), `ui/src/lib/components/DecoderPane.svelte` (prop `embedded` disables fixed positioning), keymap handler for `decode-selection` navigates to `/decoder` with the seed.
**Interfaces → Consumes:** T0 components. **Produces:** `router.go('/decoder', {seed})`-style navigation: `decoder.svelte.ts` store with `seed(bytes, nonLatin1)`; `StartPanel` props `{proxyAddr: string, apiOrigin: string, projectHint?: string}`; `ShortcutsDialog` builds rows from `keymap.bindings()` (add that accessor to `keymap.ts` returning `{action, chord, label}[]`).
- [ ] Step 1: failing tests: `router` registry includes the two routes; `keymap.bindings()` returns every default action with a label; `session.expire()` flips status; e2e `start-view.spec.ts` (fresh E2E fixture project with 0 rows shows `start-panel`; after one fixture exchange it disappears) and `shell.spec.ts` (`nav-decoder`, `nav-settings`, `?` opens `shortcuts-dialog`, theme toggle flips `html[data-theme]`, status `ws-dot` has `data-state="connected"`).
- [ ] Step 2: implement per spec D3 + D6 (Decoder/Settings routes). Where the proxy address is shown, source it from the E2E fixture / server config the UI already receives; if no field exists, show the API origin and omit the proxy chip rather than invent one.
- [ ] Step 3: lint, check, test, e2e green (runs alone). Commit `feat(ui): tool rail, top bar, start panel, decoder and settings routes`.

### Task 2: History and exchange detail (parallel worktree A)
**Files:** Modify `ui/src/routes/History.svelte`, `ui/src/lib/components/ExchangeDetail.svelte`, `ui/src/lib/stores/history.svelte.ts` (bulk tag helper), create `ui/src/lib/components/{SelectionToolbar,HeadersTable,DiagnosticsList,RowContextMenu}.svelte`, `ui/src/lib/http/headers.ts` (`parseHeadLines(bytes): {start: string, headers: {name,value}[], raw: boolean}`; never decodes, splits on CRLF/LF, lone CR kept in text, obs-fold joined), `ui/src/lib/copy-url.ts` (`copyUrl(row): string`). Tests: `headers.test.ts`, `copy-url.test.ts`, history store bulk-tag test. Must NOT edit `ui/src/lib/ui/*` or `Shell.svelte`.
**Interfaces → Consumes:** T0 components, `toasts`, `decoder.seed`. **Produces:** testids `selection-toolbar`, `row-menu`, `detail-tab-headers|raw|hex`, `diag-group` (with `data-code`, `data-count`).
- [ ] Step 1: failing unit tests (header parser cases incl. CRLF, LF, lone CR, obs-fold, non-UTF-8 bytes shown via Latin-1; copyUrl for tls/non-tls, default ports elided, query kept; bulk tag issues one PATCH per selected id in order).
- [ ] Step 2: implement spec D4 (toolbar, context menu, badges, humanised columns, split detail with Headers/Raw/Hex, grouped diagnostics, bulk actions; enable Send to Repeater and Copy as curl buttons; Hex tab windowed to 64 KiB pages with prev/next).
- [ ] Step 3: lint, check, test green (no e2e here). Commit `feat(ui): discoverable history actions and split exchange detail`.

### Task 3: Repeater (parallel worktree B)
**Files:** Modify `ui/src/routes/Repeater.svelte`, `ui/src/lib/components/{RepeaterTab,ConnSettings,ResultHeader}.svelte`, `ui/src/lib/stores/repeater.svelte.ts` (tab title derivation `titleFor(tab)`, local rename). Tests: `repeater.test.ts` additions (title from method+host, fallback `Tab N`). Must NOT edit `ui/src/lib/ui/*` or `Shell.svelte`.
- [ ] Step 1: failing tests for `titleFor` and rename.
- [ ] Step 2: implement spec D5 (toolbar with kbd hints, segmented mode, collapsible connection card, split request/response, empty state, response Headers/Raw/Hex reusing T2's `HeadersTable` — if T2 is not merged yet, implement against the interface and the controller resolves on cherry-pick).
- [ ] Step 3: lint, check, test green. Commit `feat(ui): repeater layout with visible actions and response tabs`.

### Task 4: Intercept, Scope (parallel worktree C)
**Files:** Modify `ui/src/routes/Intercept.svelte`, `ui/src/routes/Scope.svelte`, `ui/src/lib/components/{HeldList,RulesEditor,ScopeRules,ScopeImport}.svelte`. Must NOT edit `ui/src/lib/ui/*` or `Shell.svelte`.
- [ ] Step 1: no new unit logic; snapshot of testids preserved (grep test: every `data-testid` string from the pre-change files still exists after).
- [ ] Step 2: implement spec D6 (switch in view, queue badges, editor toolbar with kbd, Rules card titled exactly "Rules", Scope cards + sticky save, empty states).
- [ ] Step 3: lint, check, test green. Commit `feat(ui): intercept and scope views on the component set`.

### Task 5: integration, e2e, XSS corpus, bundle, full gate
**Files:** cherry-pick T2–T4 onto the branch; update `ui/e2e/perf-scroll.spec.ts` (scroll `history-list` container), `ui/e2e/intercept.spec.ts` (Rules card title), add `ui/e2e/discoverability.spec.ts`, extend `ui/e2e/xss-corpus.spec.ts` over Headers tab cells, detail title, repeater tab title, toast text, context-menu labels, Start panel, Settings About; `just ui-build` → commit the rebuilt bundle under `crates/crosswire-server/assets/ui/`; `docs/superpowers/specs/2026-10-25-crosswire-ui-design.md` gets a "U1 addendum" pointer; `CHANGELOG.md` entry.
- [ ] Step 1: resolve cherry-picks; `npm run lint && npm run check && npm run test`.
- [ ] Step 2: `npm run e2e` (all specs incl. XSS and zero-egress) green; fix only product bugs, never relax a detector.
- [ ] Step 3: `just ui-build`, `just ui-dist-check` (no drift), bundle delta reported against the budget (+60 KB JS / +20 KB CSS).
- [ ] Step 4: `just lint`, `just doc`, `cargo deny check`, `cargo test --workspace --locked --all-features` (background), `just first-capture-ui` if runnable locally.
- [ ] Step 5: Commit `test(ui): e2e and XSS coverage for the new surfaces; rebuilt bundle` (bundle in its own commit `build(ui): rebuild embedded bundle`), whole-branch review, PR, merge on green.
