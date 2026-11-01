# U2 Super-App Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the UI a tool a tester reaches for: a real message viewer, a working repeater bench, fast history navigation, application identity (auto-login, standalone window, project name, icon), and remappable shortcuts.

**Architecture:** A shared `MessageViewer` component (Pretty | Raw | Headers | Hex) replaces the per-view tab stacks in History detail and Repeater response; pretty views and tokenizers are pure modules over the recorded bytes. The repeater stores send history and tab names inside its existing opaque draft JSON. `serve --open`/`crosswire app` reuse the browser-detection code; health gains `project`; the bundle ships a manifest and icons. The keymap gains a persisted override layer.

**Tech Stack:** Svelte 5, CodeMirror 6 (+ `@codemirror/language` and `@codemirror/stream-parser` promoted to direct deps; both already in the lockfile), Rust std only for the server changes.

**Spec:** `docs/superpowers/specs/2026-11-01-crosswire-u2-super-app-design.md`

## Global Constraints
- `ui/package.json`: the ONLY change is adding `@codemirror/language` and `@codemirror/stream-parser` at the exact versions already present in `package-lock.json`; the lockfile's package set must not change (`npm ci --ignore-scripts` must not add or remove packages — verify by diffing `npm ls --all --parseable` before/after). No Rust dependency change.
- §11.1: pretty views, tables, diff and tokens render text only; tokenizers return CSS class names from a fixed set; no `{@html}`, innerHTML, srcdoc, dynamic href/src, `<svelte:element>`; ESLint config unchanged, no new `eslint-disable`; no `style=`/`style:`; hex colours only in theme.css.
- `--open` and `crosswire app` pass URLs as single argv entries, never through a shell; default off.
- All existing e2e test ids preserved; XSS corpus extended over every new surface (Pretty JSON/form/query tables, diff view, search box, project name/title, tab names, send history rows).
- Bundle rebuilt and `ui-dist-check` clean; budget +50 KB JS / +10 KB CSS raw; full CI gate before push; Conventional Commits ≤72; NO AI attribution trailer.

## Review Focus
1. A JSON body of 5 MB → Pretty refuses with a note and Raw shows the first 1 MiB with "Show all"; no UI freeze (T0 test with a generated body).
2. A body that is valid JSON but `content-type: text/html` → Pretty still offered (parse-detected), Raw default (T0 test).
3. Restoring a send from history after the draft was edited → editor shows the restored bytes exactly, dirty flag set, nothing sent (T1 test).
4. `crosswire serve --open` on a machine with no browser → serve keeps running, prints the URL, exit code unchanged (T3 test with `BROWSER=/nonexistent`).
5. Remapping a shortcut to Ctrl+T → refused with a message; to a chord already used → refused naming the other action (T4 test).

---

### Task 0: message viewer (opus)
**Files:** Create `ui/src/lib/viewer/{MessageViewer.svelte,PrettyJson.svelte,KvTable.svelte,FindBar.svelte}`, `ui/src/lib/viewer/{detect.ts,pretty.ts,find.ts}`, `ui/src/lib/editor/lang/{json.ts,http.ts,markup.ts}` (StreamLanguage tokenizers), `ui/src/lib/editor/highlight.css` tokens; Modify `ui/src/lib/editor/ByteEditor.svelte` (props `wrap`, `showControls`, `language`), `ui/src/lib/components/ExchangeDetail.svelte` and `RepeaterResponse.svelte` (use `MessageViewer`), `ui/package.json` (two direct deps), `theme.css` (syntax tokens). Tests: `detect.test.ts` (JSON by type / by parse / form / query; size guard), `pretty.test.ts` (stable output, 2 MiB guard, invalid JSON falls back), `find.test.ts` (Latin-1 search, case-insensitive, match count, wrap-around), tokenizer tests (JSON/HTTP/markup token classes on fixtures, hostile input never throws), component test for `KvTable` text-only rendering.
**Interfaces → Produces:** `MessageViewer {head: Uint8Array|null, body: Uint8Array, contentType?: string, query?: string, side: 'req'|'resp', testidPrefix: string, defaultTab?: 'pretty'|'raw'|'headers'|'hex'}`; tabs testids `${prefix}-tab-pretty|raw|headers|hex` (keep existing `detail-tab-*`/`resp-tab-*` names by passing the prefix); toolbar toggles `${prefix}-wrap`, `${prefix}-controls`, `${prefix}-find`.
- [ ] Step 1: failing tests above. Step 2: implement D1. Step 3: `npm run lint|check|test`, `npm run e2e` (viewer touches History/Repeater specs — fix only selectors that moved, with reasons). Commit `feat(ui): message viewer with pretty, colouring, wrap and find`.

### Task 1: repeater bench (opus, worktree)
**Files:** Modify `ui/src/lib/stores/repeater.svelte.ts` (draft record gains `history: SendRecord[]` ≤50, `title`; `restore(tabId, index)`, `compareWithPrevious`), `ui/src/routes/Repeater.svelte`, `ui/src/lib/components/RepeaterTab.svelte` (template on new tab, history strip `send-history`, Compare button), create `ui/src/lib/components/SendHistory.svelte`, `ui/src/lib/diff/lines.ts` (LCS line diff, bounded 20k lines), `DiffView.svelte`. Tests: store history/restore/rename persistence, diff correctness (add/remove/unchanged, bound), restore sets dirty without sending.
- [ ] Commit `feat(ui): repeater send history, restore and response compare`.

### Task 2: history navigation (sonnet, worktree)
**Files:** Modify `ui/src/routes/History.svelte` (search box `history-search`, `/` focus, `j`/`k`/Enter/Space/Esc, column picker `column-picker`, default split 45/55 via Splitter default ratio), `ui/src/lib/stores/history.svelte.ts` (`clientFilter`), `ui/src/lib/format.ts` (`timeCell(iso, now)`). Tests: client filter over host/path/tags, `timeCell` today vs older, keyboard navigation unit (focus index math), column visibility persistence.
- [ ] Commit `feat(ui): history search, keyboard navigation and column picker`.

### Task 3: app identity (opus, Rust + UI; main checkout)
**Files:** Rust: `crates/crosswire-server/src/serve.rs` (`--open[=browser|app]`), new `src/open.rs` (default-browser open via `open`/`xdg-open`/`cmd /c start`, app mode via the detected Chromium with `--app=URL`), `src/main.rs` + new `src/app_cmd.rs` (`crosswire app --project DIR` = serve --open=app + launch wired browser once proxy is up), `crates/crosswire-api/src/routes/misc.rs` + `app.rs` (`project: Option<ProjectInfo { name, path }>` on health), OpenAPI snapshot, `docs/api.md`, server README table; UI: `ui/public/manifest.webmanifest`, `ui/public/icons/{icon.svg,icon-192.png,icon-512.png,favicon.ico}` (drawn in-repo; PNGs generated by a tiny Node script using canvas-free SVG→PNG? — no dependency allowed: commit the PNGs produced once locally with Playwright's screenshot of the SVG, documented in the commit), `ui/index.html` links, `TopBar.svelte` project chip + `document.title`, `types.ts`, `crates/crosswire-server/src/ui.rs` (serve manifest/icons with correct content types; CSP unchanged). Tests: Rust unit for the open-command construction (argv, no shell), CLI test `serve --open` with `BROWSER=/nonexistent/false` keeps serving; health test for `project`; e2e: title and project chip from a mocked health; manifest reachable with the right content type via first-capture-ui or a server test.
- [ ] Commits `feat(server): serve --open and the app subcommand`, `feat(api): project name on /health`, `feat(ui): project identity, favicon and installable manifest`.

### Task 4: remappable shortcuts (sonnet, worktree)
**Files:** `ui/src/lib/keymap.ts` (persisted overrides, reserved-chord list, conflict detection, `reset()`), `ui/src/routes/Settings.svelte` (Change/Reset UI, `shortcut-change-<action>`), tests: unit for reserved/conflict/persist; e2e: remap Send to Repeater to Ctrl+Shift+E, reload, hint updated, old chord inert.
- [ ] Commit `feat(ui): remappable, persisted keyboard shortcuts`.

### Task 5: integration (opus)
- [ ] Cherry-pick T1, T2, T4 onto the branch after T0 and T3; resolve conflicts (RepeaterTab/ExchangeDetail touched by T0 and T1); XSS corpus over all new surfaces; e2e all green; `just ui-build` + dist check + size delta; full Rust gate incl. background workspace test and `just first-capture-ui`; screenshots of every view dark/light; whole-branch review; PR; merge.
