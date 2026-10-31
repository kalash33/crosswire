# U1: the "proper app" UI (design system, shell, discoverability)

**Status:** approved 2026-10-31 (direction: pro web UI, dark default). **Why now:** the maintainer tested the running sidecar and found the embedded web UI unfit for a public 0.1.0: browser-default controls, no theme, a clipped status bar, features reachable only by keyboard chord or hover, bare empty views. The backend, security gates and data model are release-ready; the UI is not. U1 is inserted into the 0.1.0 release tail before R3/R4. **Direction approved by the maintainer:** a polished professional web UI (Burp/Caido class) shipped in the single binary; dark theme by default with a light option; no desktop wrapper for 0.1.

**Inputs:** screenshots of the live app (2026-10-31), the UI inventory (shell/routing, stores, keymap, per-view feature discoverability, §11.1 rules, CSP, tests), the UI design spec `docs/superpowers/specs/2026-10-25-crosswire-ui-design.md` and product spec §5/§7.

## 1. Goals and non-goals

**Goals**
1. Every feature the UI already has is visible and reachable with the mouse, labelled, and shows its shortcut.
2. A coherent visual system: one token set, one component vocabulary, dark default + light, consistent spacing, type and icons, no browser-default widgets.
3. A shell that behaves like an application: fixed-height layout with a tool rail, top bar, scrolling content area and a pinned status bar; no document-level scrolling.
4. First-run guidance: a Start view that explains how to get traffic in (proxy address, CA download, browser launch command) and empty states that say what to do next.
5. Request/response reading that fits the screen: a split detail with Headers / Raw / Hex tabs, grouped diagnostics, a real title.
6. No regression of the security posture (§11.1 rendering rules, CSP, zero new runtime dependencies, bundle reproducibility) and the existing e2e contracts, updated only where the layout change requires.

**Non-goals (0.1.x or later):** virtualised history table (perf gap recorded), new API fields (MIME/TLS/ms columns need server work), response intercept (beta-2), match-and-replace (beta-3), keymap remapping UI (read-only cheat sheet only), project lifecycle (save-as/recent), browser-launch button (needs a server endpoint; U1 shows the command), web fonts (CSP `default-src 'self'` and bundle size: system font stacks are used).

## 2. Constraints carried
- Svelte 5 runes, Vite, CodeMirror 6; **no new npm dependency** (icons are inline SVG from a path map; the splitter, toasts, tabs and menus are small in-repo components).
- §11.1: every captured value is text-interpolated; no `{@html}`, innerHTML, srcdoc, dynamic `href`/`src`; the ESLint rule set is unchanged and gains no `eslint-disable`. The only URL the UI builds is from the server's own trusted config (proxy address from `/api/v1/health`-adjacent config), opened via a button handler, never an `href`.
- CSP: `style-src 'self' 'nonce-…'` so no `style=` attributes and no injected `<style>` elements; CSSOM custom-property writes (`element.style.setProperty('--x', …)`) are permitted (not governed by `style-src`); theming is CSS custom properties toggled by a `data-theme` attribute on `<html>`; `default-src 'self'` so no CDN assets.
- `proxy_addr` on `GET /api/v1/health` (added in a separate PR) feeds the Start panel, Settings and the status bar; until a server reports it (or when the value fails `parseProxyAddr`), the UI shows the `crosswire serve` default and the status bar shows no proxy item.
- Bundle rebuilt and committed (`just ui-build`), `just ui-dist-check` green; full CI gate; Conventional Commits; no AI trailers.
- The `data-testid` contract the e2e specs depend on is preserved; where the layout change moves a control (status-bar intercept toggle, `<summary>Rules`, document scrolling in `perf-scroll`), the spec is updated in the same task with the reason.

## 3. Design

### D1 Design tokens and theme (`ui/src/lib/theme.css`, imported by `app.css`)
- `:root` defines the dark theme: surfaces (`--bg`, `--bg-elev`, `--bg-sunken`), borders, text (`--fg`, `--fg-muted`, `--fg-faint`), accent (`--accent`, `--accent-fg`), semantic (`--ok`, `--warn`, `--danger`, `--info`), method colours (GET/POST/PUT/DELETE/PATCH/other), status colours (2xx/3xx/4xx/5xx), focus ring, radii (`--r-1: 4px`, `--r-2: 6px`), spacing scale (`--s-1: 4px` … `--s-6: 24px`), type (`--font-ui: system-ui…`, `--font-mono: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace`, sizes 12/13/14/16).
- `:root[data-theme="light"]` overrides the colour tokens only. Default is dark; `ui/src/lib/theme.ts` reads/writes `localStorage["crosswire.theme"]` (try/catch) and sets the attribute before first paint (inline in `index.html` is not allowed by CSP, so `main.ts` applies it synchronously before mounting; a one-frame flash is acceptable).
- Every component's scoped CSS uses tokens only; the ~30 hard-coded hex values are removed (a lint-ish unit test greps `ui/src` for `#[0-9a-f]{3,8}` outside `theme.css` and fails on new ones).

### D2 Component vocabulary (`ui/src/lib/ui/`)
`Button.svelte` (variants: primary, default, ghost, danger; sizes sm/md; optional leading `Icon`; `kbd` hint prop renders the shortcut), `IconButton.svelte` (always has `aria-label` + tooltip), `Icon.svelte` (name → inline `<svg><path d=…>` from `icons.ts`, ~24 icons: history, repeater, intercept, scope, decoder, settings, send, copy, tag, plus, close, search, filter, chevron, play, pause, forward, drop, hex, raw, headers, sun, moon, keyboard, external), `Input.svelte`/`Select.svelte` (styled natives), `Segmented.svelte` (Assisted/Raw, Headers/Raw/Hex), `Tabs.svelte`, `Badge.svelte` (method, status, state, scope), `Chip.svelte` (tags, assisted chips), `Toolbar.svelte`, `EmptyState.svelte` (icon, title, body, actions), `Kbd.svelte`, `Menu.svelte` (context/overflow menu, keyboard navigable, Escape closes, items are buttons), `Splitter.svelte` (pointer-driven resizable two-pane, min sizes, persists ratio in localStorage), `Toast.svelte` + `toasts.svelte.ts` store (info/success/error, auto-dismiss, `role="status"`), `Tooltip` via `title` plus a visible `Kbd` where a shortcut exists.

### D3 Shell (`Shell.svelte`, `StatusBar.svelte`, `TopBar.svelte` new, `Rail.svelte` new)
- Layout: `.shell { display:grid; grid-template-columns: 56px 1fr; grid-template-rows: 44px 1fr 28px; height: 100dvh; overflow: hidden }`. `.app-root` loses its padding. Content area scrolls internally; nothing scrolls the document.
- **Rail** (left): icon + label buttons for History, Repeater, Intercept, Scope, Decoder, Settings; active state; `nav-*` testids preserved; `nav-decoder`, `nav-settings` added. Routes `/decoder` and `/settings` added to the router registry.
- **Top bar:** project directory name (from a new `GET /api/v1/health`-adjacent field? no server change: the UI shows the API origin and proxy address it already knows; project name is deferred), proxy address chip, intercept switch (mirrors the status-bar toggle; both call the same store), theme toggle, "Shortcuts" (`?`) button opening the cheat sheet, WS connection dot.
- **Status bar** (pinned): API origin, proxy address (replaces "Proxy n/a" with the value the server reports; if unknown, omit), WS connected/disconnected, Intercept ON/OFF with the existing `intercept-toggle` and `status-intercept` testids, scope state, counts. Single line, no wrap, overflow hidden.
- **Shortcut cheat sheet:** modal listing every keymap action with its chord (from `keymap.ts` registry via `chordFor`), opened by `?` or the top-bar button, closed by Escape.
- **Toasts** for copy results (replacing inline `copy-status` text is NOT done: the `copy-status` testid stays and the toast is additional), save errors, send errors, intercept forward/drop results.
- **Auth:** a mid-session 401 from any API call sets `session.status = 'unauthenticated'` and shows the Auth screen (restyled with the same components, bootstrap-URL paste kept).
- **Start view:** when History is empty and no filter is applied, the History view shows a Start panel instead of an empty table: "Proxy listening on `127.0.0.1:8080`" (copy button), "Trust the CA" with the CA page address and `ca.pem`/`ca.der` names (copy address button; "Open CA page" button uses `window.open` on the server-reported proxy address), and "Launch a wired browser" showing the `crosswire browser --project … --proxy …` command to copy. The panel disappears as soon as the first exchange arrives (WS `received`).

### D4 History
- Toolbar above the table: a compact filter bar (host, contains, method, status range, tag, state, scope) collapsible behind a Filter button with an active-count badge; Apply/Clear kept with testids; "N loaded · N selected" + Load more at the bottom of the scroll container.
- Table: method as a coloured `Badge`, status as a coloured `Badge`, length humanised (`1.8 MB`), time as `HH:MM:SS` with full timestamp on hover, path truncated with full value on hover, tags as chips, uncommitted dot kept; `td.open`/`td.tags` classes and `history-row` testid kept.
- Row actions: a **selection toolbar** appears above the detail when a row is open (Send to Repeater `Ctrl+R`, Copy menu, Tag, Add host to scope, Preview) and the same items are offered in a **right-click context menu** on a row (`Menu.svelte`). The disabled "Available in a later release" stubs are replaced by the working actions (send-to-repeater and copy already work via chords). "Copy URL" is added client-side (scheme from tls flag, host, path; plain text).
- Bulk: with ≥1 rows checked, the toolbar offers "Tag selected…" (sequential PATCHes through the existing per-field lane) and "Add hosts to scope" (unique hosts). Select-all kept.
- **Detail pane** (right of a `Splitter`): header = `METHOD /path` (monospace), host, status badge, state and scope badges, length and time; tags/notes editor below the header (testids kept). Then a Request | Response segmented view with a vertical `Splitter` (side-by-side on ≥1280px, stacked below). Each side has sub-tabs **Headers** (parsed lines, name/value table; CR/LF never rendered), **Raw** (the existing read-only `ByteEditor`, placeholders kept), **Hex** (`HexPane` over head+body bytes). Diagnostics render as one collapsible row per code with a count badge (`7× non_canonical_header_case`) and the span list inside; severity colours by token. Copy menu and Preview button live in the Response/Request toolbars; `detail-*`, `copy-*`, `preview-*` testids kept.

### D5 Repeater
- Tab strip with real titles: `METHOD host` derived from the draft (fallback `Tab N`), close icon, `+` New; `repeater-tab` testid kept. Double-click renames locally (in-memory).
- Two-pane `Splitter`: request editor (left) with a toolbar: Send (primary, `Ctrl+Space` kbd), Paste raw, Copy raw, Decode selection, Assisted/Raw `Segmented`, assisted options as chips with checkboxes; connection settings in a collapsible card above the editor (host/port/TLS/SNI/verify/reuse/follow), styled with the component set, `conn-*` testids kept.
- Response (right): empty state "Send the request to see the response" before the first send; after: result header as a status line (status badge, TTFB, total, size, scope, cert-override badge, change chips) and Headers/Raw/Hex sub-tabs; `result-*` testids kept.

### D6 Intercept, Scope, Decoder, Settings
- **Intercept:** a prominent switch at the top of the view (same store as the status-bar toggle, same testids on the status bar preserved), queue list with method/host/path badges, "Forward all"; selected item opens the editable `ByteEditor` with a toolbar Forward (`Ctrl+F`), Drop (`Ctrl+D`), Send to Repeater (`Ctrl+R`); Rules in a right-hand card (always visible, no `<details>`; the e2e `getByText('Rules')` click is updated to the card title, which stays exactly "Rules"); empty state "Intercept is off" / "Nothing held".
- **Scope:** two cards (Include / Exclude) with rule rows restyled, a sticky Save bar, the strict switch as a labelled switch with the server-state hint, Import as a third card with preview list; all testids kept.
- **Decoder:** becomes a route `/decoder` with the existing pane embedded full-height; "Decode selection" buttons in the History detail and Repeater toolbars seed it (the chord keeps working from anywhere and now navigates to `/decoder`); the fixed overlay is removed; `decoder-*` testids kept.
- **Settings:** route `/settings`: Appearance (theme), Shortcuts (read-only table from the keymap registry), Connection (API origin, proxy address, CA page address, copy buttons), About (app version from `/api/v1/health` if present, `schema_version`, `format_version`, links as literal `href` constants to the GitHub repo).

### D7 Tests
- Unit: tokens-only test (no hex outside theme.css), `theme.ts` persistence, `Menu` keyboard navigation, `Splitter` clamping, toast store, `humanBytes`/`timeShort` helpers, header-line parser for the Headers tab (CRLF/LF/lone CR, obs-fold line counted as continuation, no decoding).
- e2e (updated): `perf-scroll` scrolls the history container not the window; `intercept.spec` clicks the Rules card title; `mouse-free` unchanged (chords kept); new `discoverability.spec.ts` (every rail item reachable; row toolbar shows Send to Repeater and it opens a tab; context menu copies URL; decoder button seeds the pane; `?` opens the cheat sheet; theme toggle flips `data-theme`), `start-view.spec.ts` (empty project shows the Start panel, first exchange hides it). XSS corpus extended over the new surfaces: Headers tab name/value cells, detail title, repeater tab title, toast text, context-menu labels, Start panel values (server-reported addresses), Settings About fields. Zero-egress preview spec unchanged.
- Gate: `just ui-lint`, `just ui-test`, `npm run e2e`, `just ui-build` (bundle committed), `just ui-dist-check`, then `just lint`, `just doc`, `cargo deny check`, `cargo test --workspace` (server embeds the bundle; the first-capture-ui config runs against the real server).

## 4. Decomposition (one plan, six tasks)
T0 tokens + component vocabulary + shell layout fix (foundation, sequential) → T1 shell: rail, top bar, status bar, toasts, cheat sheet, auth, Start view (sequential) → T2 History + detail, T3 Repeater, T4 Intercept/Scope/Decoder/Settings (parallel in worktrees, each gated by lint + vitest + svelte-check, no e2e) → T5 integration: merge, e2e updates and new specs, XSS corpus, bundle rebuild, full gate, whole-branch review.

## 5. Risks
- Bundle growth: icons and components add CSS/JS; budget ≤ +60 KB JS, ≤ +20 KB CSS (measured in T5; the dist check pins bytes, not size).
- The History table is still not virtualised; the redesign must not make the 1M-row scroll worse (perf-scroll stays report-only and is re-measured).
- Theme flash on first paint (CSP forbids inline scripts); accepted.
- Three parallel UI tasks touching shared components: T0/T1 own `ui/src/lib/ui/*` and `Shell.svelte`; T2–T4 must not edit those files (they request additions through the controller), which keeps the cherry-picks conflict-free.
