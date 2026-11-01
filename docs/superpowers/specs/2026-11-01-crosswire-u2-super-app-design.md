# U2: the "super app" pass (viewer, repeater, history, app identity)

**Status:** approved 2026-11-01. **Why:** after U1 the UI is coherent but not yet a tool a tester would reach for over Burp/Caido. The maintainer's hands-on test and the controller's own pass over the live build agree on the gaps: a weak message viewer, an empty repeater, a list without search, and no application identity (60-second login dance, no project name, no icon, nothing opens the UI for you). U2 closes those before `v0.1.0`; it is inserted before R4 docs/launch so the docs describe the final product.

## 1. Goals
1. Reading a message is pleasant: pretty views for JSON, form bodies and query strings, syntax colouring, word-wrap and control-character toggles, find-in-body, and a Raw tab that is clean by default.
2. The repeater is a working bench: a new tab starts from a template or the selected exchange, every send is kept in a per-tab history you can restore, and two responses can be compared.
3. History is navigable at speed: a search box, keyboard row navigation (`j`/`k`/`Enter`), column visibility, a sensible default split, and the date on hover.
4. Crosswire behaves like an application: `crosswire serve --open` opens the UI already logged in; the server-reported project name sits in the top bar and the window title; a favicon and a web-app manifest make it installable as a standalone window from Chrome/Edge; a `crosswire app` subcommand launches that standalone window directly.
5. Shortcuts are remappable and persisted.

## 2. Non-goals (0.1.x or later)
Native desktop wrapper (Tauri) — the installable PWA window plus `crosswire app` gives the app experience without a new Rust dependency or code signing; response intercept (beta-2) and match-and-replace (beta-3); virtualised history table (perf gap, separate); a full HTML/JS beautifier (JSON, form and query pretty views only; HTML gets colouring, not reformatting).

## 3. Decisions

### D1 Message viewer (History detail and Repeater response)
- **Tabs become Pretty | Raw | Headers | Hex.** Pretty is offered when the body is JSON (`application/json`, `+json`, or a body that parses as a JSON object or array), `application/x-www-form-urlencoded` (a key/value table, URL-decoded, with the raw string underneath), or when the request has a query string (a key/value table above the body). **Default tab:** Pretty when the declared type says JSON or form, or when the body parses as JSON and declares no other type (empty or `text/plain`), or for a query string; otherwise Raw (a JSON-looking `text/html` body opens on Raw with Pretty offered).
- **Syntax colouring** in Raw/Pretty via CodeMirror `StreamLanguage` with hand-written tokenizers for JSON and HTTP heads (status line, header names, values) and a minimal HTML/XML tag tokenizer. The tokenizers return names from a fixed set; a class-only decoration plugin turns the syntax nodes into fixed `tok-*` classes whose colours are `--syn-*` theme tokens (no `HighlightStyle`: it needs `@lezer/highlight`, which is not a declared dependency, and the plugin injects no style at all). `@codemirror/language` and `@codemirror/streamparser` (the latter types-only, never imported directly) are already in the lockfile as transitive dependencies; they become **direct** dependencies in `package.json` without changing the lockfile's package set (ruling requested in the approval: this adds no new code to the bundle's provenance).
- **Toggles** in the viewer toolbar: word wrap (default on for bodies, off for heads), show control characters (default off: CR/LF placeholders hidden in Raw, shown as `␍␊` when on), find-in-message over the head and body (`Ctrl+F` inside the viewer; own incremental search over the Latin-1 text with match count, capped at 10,000 matches shown as `10000+`, and next/prev; no CodeMirror search package).
- Pretty views are derived in the UI from the recorded bytes and never sent anywhere; the byte-exact Raw tab stays the source of truth. Pretty JSON validates with `JSON.parse` and then re-indents the **recorded text** (no `JSON.stringify` round trip, so number literals, escapes, key order and duplicate keys are kept), with a size guard (over 2 MiB → Raw only with a note). A body that is not strict UTF-8 shows a note in Pretty (replacement characters). Form and query tables render at most 1,000 rows with a "Showing 1000 of N" note.
- Large bodies: Pretty renders in full up to the 2 MiB guard (the editor virtualises lines); Raw renders the first 1 MiB and offers "Show all"; Hex is already paged.

### D2 Repeater bench
- **New tab** seeds `GET / HTTP/1.1\r\nHost: \r\n\r\n` and focuses the Host field; "New from selection" in History (existing Send to Repeater) keeps seeding from the exchange.
- **Send history per tab:** every send appends `{at, status, ttfb, total, size, request bytes digest, exchange id}` to the tab's draft record (persisted via the existing draft PUT; the server already stores drafts as opaque JSON); a History strip under the tabs lists the last 50 with status badges; clicking restores that request into the editor and shows its response (fetched by exchange id).
- **Compare:** "Compare with previous" renders a line diff (dependency-free LCS over lines, bounded to 20k lines) of the two responses' Raw text, with added/removed lines coloured by token; headers and body in one text.
- **Tab titles** derive from `METHOD host` (U1) and persist renames into the draft record.

### D3 History navigation
- **Search box** in the toolbar (`/` focuses it): a client-side substring filter over host, path and tags of the loaded rows, combined with the server filter bar (which stays authoritative for the data set). Shows "N of M loaded match".
- **Keyboard:** `j`/`k` (and arrow keys) move the focused row, `Enter` opens, `Space` toggles the checkbox, `Esc` returns focus to the list; focus ring visible.
- **Columns:** a column-picker menu (Time, Method, Host, Path, Status, Length, State, Tags; Host and Path cannot be hidden) persisted in localStorage; the Time cell shows the date in `title` and switches to `MMM D HH:MM:SS` when a row is older than today.
- **Default split** list 45% / detail 55% (first run), persisted thereafter.

### D4 Application identity
- **`crosswire serve --open`** (and `--open=app`): after the API binds, open the bootstrap URL in the default browser (`open`/`xdg-open`/`cmd /c start` via `std::process::Command`, no dependency) or, with `--open=app`, in a Chromium-based browser's app mode (`--app=<bootstrap url>` reusing the browser-detection code of `crosswire browser`), so the user never copies the 60-second token. Default stays off (scripts and CI unchanged). The URL is passed as a single argument, never through a shell.
- **`crosswire app --project DIR`**: convenience = `serve --open=app` plus launching the wired Chromium (`browser`) once the proxy is up; one command gives the whole workstation.
- **Project identity:** `/api/v1/health` gains `project: { name, path }` (directory basename and absolute path; health is loopback-only); the top bar shows the name with the path in `title`; `document.title` becomes `name · view · Crosswire`.
- **Installable:** `ui/public/manifest.webmanifest` (`display: standalone`, theme/background colours from tokens, icons) and an SVG + PNG icon set drawn in-repo (no asset downloads); `<link rel="icon">`/`manifest` in `index.html`; the server serves them with the bundle (same CSP; manifest is `application/manifest+json`). Chrome/Edge then offer "Install Crosswire" and open it as its own window.

### D5 Remappable shortcuts
Settings → Keyboard shortcuts: each row gets a "Change" button that records the next chord (modifier + key), rejects browser-reserved chords (Ctrl+T/N/W and their Shift variants) and conflicts, and persists to `localStorage["crosswire.keymap"]` (try/catch); `keymap.remap()` becomes the single source, hints update live, "Reset to defaults" button. e2e covers remap + persistence + reserved-chord refusal.

### D6 Constraints
§11.1 unchanged (pretty views are text; tokenizers colour via CSS classes, never HTML); CSP unchanged; the only `package.json` change is promoting the two transitive CodeMirror packages to direct dependencies (lockfile package set unchanged, `npm ci --ignore-scripts` unaffected); Rust changes are small (`--open`, `app` subcommand, `project` on health) with no new crate dependency; bundle rebuilt and drift-checked; all existing e2e test ids preserved; XSS corpus extended over Pretty views, the form/query tables, the diff view, the search box, the project name and window title.

## 4. Tasks (plan to follow on approval)
T0 viewer (opus) → T1 repeater bench (opus) → T2 history navigation (sonnet) → T3 app identity: Rust `--open`/`app`/`project` + manifest/icons/title (opus; Rust + UI) → T4 remappable shortcuts (sonnet) → T5 integration, XSS, bundle, full gate, screenshots. T1–T4 in parallel worktrees after T0's viewer lands (T1 reuses the viewer).

## 5. Risks
Bundle growth (tokenizers, diff, tables: budget +50 KB JS, +10 KB CSS); the app-mode launch depends on a Chromium-based browser being installed (falls back to the default browser with a note); `--open` on headless CI is never used (off by default).
