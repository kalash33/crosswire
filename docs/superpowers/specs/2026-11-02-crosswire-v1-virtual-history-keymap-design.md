# V1: virtualised History and a scoped keymap (Crosswire 0.1.1)

**Status:** scope approved by the maintainer 2026-11-02; this document carries every decision so implementers do not guess. **Why:** the §13 UI target ("no frame > 32 ms in a Playwright trace over a 10 s scroll, filter applied < 300 ms at 1M rows", product spec `2026-10-22-crosswire-design.md` §13) is the one §13 performance row still REPORT-ONLY. Since the U2 Load-more fix made the perf harness actually page, the History list is measurably worse: at 100k rows `max_frame` 199.9 ms, p99 150 ms, 76/356 frames > 32 ms, 3,800 rows / 63,855 DOM nodes after 10 s, filter 190 ms. Every loaded row stays in the DOM. Separately, the U2 keymap is one flat namespace: the message viewer's `Ctrl+F` is a hand-written listener that swallows the key before the dispatcher sees it (so it can coexist with Intercept's Forward on the same chord), it is not remappable, and several hints are read once at render and go stale after a remap.

**Base:** branch `feat/v1-virtual-history-keymap` from `main` after U2 (`feat/u2-super-app`) merges. Every file and line referenced below is as of the U2 tip `c691e6f`.

## 1. Goals
1. History renders only the rows in view: the DOM row count stays bounded however many rows are loaded, and scrolling meets the §13 frame target at 100k (CI, asserted) and at 1M (maintainer run).
2. Paging stays keyset: scrolling near the end loads the next page by itself, and the **Load more** button stays for keyboard and screen-reader users.
3. Nothing a user can do in U2 History breaks: search, sort, column picker, multi-select and bulk actions, the roving-tabindex keyboard model (`j`/`k`/arrows/`Enter`/`Space`/`Esc`), the context menu, live `received`/`committed` updates, and every pinned e2e selector.
4. Shortcuts have scopes (global, History, Repeater, Intercept, message viewer). Find in the viewer becomes a real, remappable `viewer`-scoped action (`Ctrl+F`), so it shares its chord with Intercept's Forward by design rather than by a listener trick. Conflicts are checked per scope. Settings and the `?` sheet group shortcuts by scope. Every hint follows remaps live.
5. U2 persisted overrides (`localStorage["crosswire.keymap"]`) load unchanged, with the same behaviour they had in U2.

## 2. Non-goals (later)
Variable-height or wrapping rows (rows are single-line, §3.1.3); a canvas or WebGL list; server-side search (search stays over loaded rows); a "jump to row N of 1M" scrollbar over unloaded rows (the scrollbar covers loaded rows only); shift-click range selection; drag-to-reorder or resize columns; Meta/Cmd chords (the keymap still never matches Meta); a keymap for Scope, Decoder or Settings views; chord sequences (`g g`); any Rust, API, store or schema change; any new npm dependency.

## 3. Decisions

### 3.1 Virtualised History list (D1)

#### 3.1.1 Approach: a native `<table>` with two spacer rows
- **Kept:** the `<table>` inside `div.list[data-testid="history-list"]`, the `<colgroup>`, the sticky `<thead>`, the sticky footer `div.foot`. No CSS-grid rewrite: the table gives column alignment with the header for free, keeps `aria-sort` on `th`, and keeps every pinned selector (`tr[data-testid="history-row"]`, `td.open`, `td.tags`, `td.time`, `td.host`, `col.c-*`) byte-identical.
- **`<tbody>` content** is `[pad-top] + window rows + [pad-bottom]`:
  - `<tr class="pad" aria-hidden="true" data-testid="history-pad-top"><td colspan={colCount}></td></tr>` and the same with `history-pad-bottom`. Each pad `td` has `padding: 0; border: 0; height: var(--vt-before)` / `var(--vt-after)` in the component's stylesheet.
  - `--vt-before` and `--vt-after` are written with `listEl.style.setProperty('--vt-before', `${px}px`)` (CSSOM; CSP-clean, the pattern `Splitter.svelte` and `Menu.svelte` already use). The values are integers computed from row indices and the measured row height, never from row data. No `style=` attribute, no `style:` directive.
  - Pad rows have no `data-testid="history-row"`, no `tabindex`, no `data-id`, so every count over `history-row` counts real rows only.
- **Layout becomes `table-layout: fixed`.** With auto layout the column widths would follow whichever rows happen to be in the window (an `OPTIONS` badge or a long date scrolls in and every column jumps), and auto layout costs a full-table measure. Fixed widths, set on the `<col>` elements in CSS:

  | col | width | worst case it must fit unclipped (discoverability "fits" spec) |
  |---|---|---|
  | `c-sel` | `2rem` | checkbox |
  | `c-idx` | `6ch` (tabular numerals) | `50000` (the row cap, §3.1.7) |
  | `c-time` | `15ch` wide / `8.5ch` under `@container (max-width: 720px)` | `Oct 31 13:52:30` with the uncommitted dot; narrow: clock only |
  | `c-method` | `6rem` | `OPTIONS` badge |
  | `c-status` | `3.5rem` | `503` badge |
  | `c-len` | `5.5rem` | `1.0 MB` |
  | `c-state` | `6.5rem` | `committed` badge |
  | `c-tags` | `7rem` (narrow `6rem`) | one chip + `+N` |
  | `c-host` / `c-path` | `40%` / `60%` of the rest (was 35/65: at 1440×900 that left Host under 60 px; 40% keeps Host ≥ 60 px there, as `discoverability.spec.ts` now asserts) | ellipsis, full text in `title` (unchanged) |

  The exact `rem`/`ch` values are tuned by the implementer until `discoverability.spec.ts` "History fits the default split" passes at 1440×900 and 1280×720 unchanged; that spec is the acceptance for widths, not this table.
- **Sticky header:** unchanged CSS (`thead th { position: sticky; top: 0 }`); the pads live inside `tbody`, so the header never scrolls away. The header height `headH` and footer height `footH` are measured once with the row height (§3.1.2) and used for reveal math.

#### 3.1.2 Row height: fixed, measured once
- Rows are single-line by construction (§3.1.3). CSS gives data rows a declared height, `tbody tr.row { height: var(--history-row-h) }` with `--history-row-h: 26px` declared on `.list` in History's own `<style>` block.
- The authoritative height is **measured**: after the first window renders, read `getBoundingClientRect().height` of the first `history-row` once (and `thead` and `.foot` heights) and store `rowH`, `headH`, `footH`. Fractional values (browser zoom) are kept as floats.
- Re-measure only when the list's `ResizeObserver` fires (split drag, window resize, container-query switch, zoom) — one read in the next rAF, never inside the scroll path. Until the first measure, `rowH` is the CSS token value.

#### 3.1.3 Single-line rows, ellipsis everywhere
- Every `td` keeps `white-space: nowrap; overflow: hidden; text-overflow: ellipsis` (already so). The Tags cell keeps one chip plus `+N` in the narrow layout and a `max-width` chip strip in the wide one; chips never wrap (`.chips { flex-wrap: nowrap }`, explicit). Full values stay in `title`. A row can therefore never be taller than `rowH`; the guard is the e2e check in §3.3.3 that every rendered row's height equals `rowH` ± 0.5 px, including rows carrying the XSS corpus (long, multi-line payloads) in host, path and tags.

#### 3.1.4 The window
- Pure module `ui/src/lib/components/virtual-window.ts` (no Svelte, unit-tested):
  ```ts
  export const OVERSCAN = 8;            // rows rendered above and below the viewport
  export const AUTOLOAD_MARGIN = 200;   // rows before the loaded end that trigger the next page
  export interface Win { start: number; end: number; before: number; after: number } // [start,end), pad px
  export function windowFor(scrollTop: number, viewportH: number, rowH: number, headH: number, total: number, overscan?: number): Win;
  export function revealTop(index: number, scrollTop: number, viewportH: number, rowH: number, headH: number, footH: number): number | null; // new scrollTop or null if already fully visible
  export function anchorAt(scrollTop: number, rowH: number, headH: number, ids: readonly string[]): { id: string; offset: number } | null;
  export function restoreAnchor(anchor: { id: string; offset: number }, indexOf: (id: string) => number, rowH: number, headH: number): number | null;
  export function shouldAutoLoad(win: Win, total: number, s: { hasMore: boolean; loading: boolean; searching: boolean; capped: boolean }): boolean;
  ```
- `first = floor(max(0, scrollTop − headH) / rowH)`, `visible = ceil(viewportH / rowH)`, `start = max(0, first − OVERSCAN)`, `end = min(total, first + visible + OVERSCAN)`, `before = start × rowH`, `after = (total − end) × rowH`. `start` is clamped so that `start ≤ total` (a data-set shrink never renders an empty window above the content).
- **Bound:** rendered rows ≤ `visible + 2 × OVERSCAN`. The acceptance bound (§3.3) is `dom_rows ≤ 2 × visible + OVERSCAN`, which holds for any viewport of ≥ 8 rows.
- `windowRows = $derived(visibleRows.slice(win.start, win.end))`, rendered with `{#each windowRows as row, j (row.id)}`; the `#` cell shows `win.start + j + 1`.

#### 3.1.5 Scroll handling
- One passive `scroll` listener on `listEl` sets a flag and requests **one** `requestAnimationFrame`; the rAF callback reads `scrollTop` (the only layout read on the scroll path; `clientHeight` comes from the `ResizeObserver` cache), computes the window, and writes state only when `start`/`end` changed (writes `--vt-before`/`--vt-after` in the same callback). Nothing in the scroll path reads row geometry.
- The same callback updates the live-prepend anchor (§3.1.9), evacuates focus if the focused row is leaving the window (§3.1.8), and runs `shouldAutoLoad` (§3.1.6).
- `ExchangeDetail` and the split pane are unaffected; the detail pane is outside `listEl`.

#### 3.1.6 Keyset paging: auto-load plus Load more
- **Auto-load** fires only from the scroll callback, when `shouldAutoLoad` is true: `win.end ≥ total − AUTOLOAD_MARGIN`, `hasMore`, not `loading`, no client search active (search is over loaded rows; auto-loading the whole project to find matches is a different feature), and not `capped`. It never fires on mount, on filter apply, or from a live event. (Consequence: a first page that does not fill the viewport does not auto-load; the user scrolls or presses Load more. The `history-nav` "Load more" spec, 3 rows then a click, stays valid.)
- **Load more** stays in the sticky footer with `data-testid="load-more"` and the same labels (`Load more` / `Loading…` / `End`), plus `Limit reached` (disabled) at the cap. It is the accessible path; auto-load is a convenience.
- **Store changes** (`ui/src/lib/stores/history.svelte.ts`):
  - `loadMore()` returns immediately while a fetch is in flight (today two calls with the same cursor would append the page twice and crash the keyed `#each` with a duplicate key).
  - Append dedupes by id against the loaded rows (insurance carried from the Plan 5a backlog).
  - `rows` becomes `$state.raw` (every update already replaces the array and the changed row object; deep proxies over 50k rows cost on every sort/filter pass).
  - A `generation` counter increments on every replace (`applyFilter`, `reload`), not on append/prepend; History resets `scrollTop` to 0 and drops the anchor when it changes.
  - `indexOf(id)` / `byId(id)` backed by a `Map` rebuilt per `rows` change (O(n) once instead of O(n) per lookup in `rowById`, `rovingId`, `moveFocus`).

#### 3.1.7 Row cap (`maxRows`)
- `maxRows` default rises from 5,000 to **50,000** loaded rows and now bounds **both** paging and live prepends (today it bounds only prepends, and its `slice(0, maxRows)` silently drops paged tail rows while `next_cursor` still points past them, so the next Load more leaves a hole).
- Paging: when `rows.length ≥ maxRows`, `capped` is true, `loadMore()` is a no-op, the button reads `Limit reached`, and a footer note `data-testid="history-cap-note"` says "50,000 rows loaded. Narrow the filter to see older exchanges." A page that would cross the cap is appended whole (page size 100 divides the cap, so in practice it lands exactly on it).
- Live prepends at the cap trim the tail as today. Because Load more is disabled once capped, no later page can resume past a trimmed tail, so the hole cannot occur. Trimmed ids leave `selected`; a trimmed focused row falls back per the roving rule; the open detail stays open (it is keyed by id, not by row).
- Why 50,000: memory (~20 MB of row objects), sort/filter latency over the loaded set (§3.1.10), and the scroll height (50,000 × 26 px = 1.3 M px, far under Chrome's ~33.5 M px and Firefox's ~17.9 M px element-height limits; an uncapped 1M-row list at 26 px would exceed Firefox's).

#### 3.1.8 Keyboard, focus and roving tabindex
- **Delegation:** the per-row `onkeydown` closures go; one `keydown` handler on `listEl` resolves the row from `e.target.closest('tr[data-testid="history-row"]')`, or uses `rovingId` when the target is `listEl` itself. Same guards as U2: no Ctrl/Alt/Meta; only the row or its checkbox (or the list) as target, so keys typed in fields never move rows.
- **Keys:** `j`/`ArrowDown`, `k`/`ArrowUp` (±1, clamped, no wrap, via `nextIndex`), `Enter` opens, `Space` toggles the checkbox (row target only, as today), and new for long lists `Home`/`End` (first/last loaded row) and `PageDown`/`PageUp` (± `visible − 1` rows). `Esc` from search and detail returns focus to the roving row (as today, through `focusList`).
- **Moving to a row outside the window:** compute the target index in `visibleRows`; `revealTop` gives the new `scrollTop`; set it, compute the window **synchronously** from it (do not wait for the scroll event), `await tick()`, then `focus({ preventScroll: true })` the row element. Focus is logical (by id), the scroll follows it.
- **Focused row leaving the window** (wheel or scrollbar while a row has focus): before the window update removes it, the scroll callback moves focus to `listEl` with `preventScroll` and sets `evacuated = true`; `focusedId` is kept. While `evacuated`, the next window that contains `focusedId` refocuses that row (`preventScroll`) and clears the flag. A key on `listEl` acts on `focusedId` (revealing it first). Focus is never lost to `<body>`.
- **Tab stop:** exactly one tab stop in the list, as today: the roving row (`tabindex=0`) when it is in the window; otherwise `listEl` itself gets `tabindex=0` (else `-1`), and focusing it reveals and focuses the roving row. The `history-nav` Tab-walk assertion (one stop, `ID[0]`) is unchanged.
- **Context menu:** `closeMenu()` returns focus to the row element only if it is still connected, else to `listEl` (`evacuated` semantics).

#### 3.1.9 Live `received` events without jumping the viewport
- **Coalescing:** the store queues live events and applies them once per animation frame (an injectable `schedule` option, default `requestAnimationFrame` with a `setTimeout(16)` fallback; unit tests inject a synchronous scheduler). A flood of 500 events costs one `rows` update and one window pass per frame, not 500.
- **Anchor by id:** the scroll callback records `anchor = anchorAt(...)` = the first fully visible row id and its pixel offset within the viewport. When `visibleRows` changes without a `generation` change (live prepend, `committed` update, page append, bulk-tag patch), a `$effect.pre` computes `restoreAnchor` and, if the anchored row moved, sets `scrollTop` and the window in the same flush, before paint. If the anchor row is gone (trimmed, filtered out) the scroll position is kept as is.
- **At the top:** when `scrollTop < rowH / 2` the list is "following the head": no anchoring, new rows appear at the top (the U2 behaviour a user watching traffic expects).
- Sorting by another column places new rows at their sort position; anchoring covers that too.

#### 3.1.10 Search, sort and selection over the loaded set
- **Search** stays `clientFilter(sortedRows, query)` over **all loaded rows**; its result array feeds the window. A query change resets `scrollTop` to 0. The footer text stays `N of M loaded match · K selected` / `M loaded · K selected` (pinned by `history-nav`).
- **Sort:** the default order (Time, newest first) uses the store order directly (server `ORDER BY ulid DESC` plus newest-first prepends; identical to `created_at` descending except for same-millisecond ties). Any header sort copies and sorts once per `rows`/sort change (≤ 50k rows, ~30–50 ms on a click, outside the scroll gate). A sort change resets `scrollTop` to 0.
- **Selection** is the U2 `SvelteSet<string>` of ids, independent of the DOM: a row scrolled out of the window stays selected and its checkbox renders checked when it scrolls back. **Select all** keeps U2 semantics: every row of `visibleRows` (all loaded rows matching the search), not just the window; the header checkbox is checked when all of them are. Selection is still pruned when a search hides a row (U2 rule: a hidden row cannot be seen or unchecked) and when the cap trims a row; it is **not** pruned when a row merely leaves the window.
- **Column picker:** unchanged behaviour and selectors (`column-picker`, `column-menu`, `column-<key>`, `col.c-*`); toggling columns does not change row height, so no re-measure is needed beyond the `ResizeObserver` path.

#### 3.1.11 Screen-reader semantics
- The `<table>` gets `aria-rowcount`: `visibleRows.length + 1` (the header row counts) when `!hasMore || capped`, else `-1` (ARIA: total unknown). The header `tr` gets `aria-rowindex="1"`; each data row gets `aria-rowindex={win.start + j + 2}`. Pad rows are `aria-hidden="true"` and carry no `aria-rowindex`. Each row keeps its U2 `aria-label`.
- The footer count and the Load more button are the accessible statement of "how much is loaded"; nothing is announced on scroll.

#### 3.1.12 Pinned selectors and which specs change
Every selector keeps its element and meaning: `history-row` (real rows only), `data-id`, `td.open`, `td.tags`, `td.time`, `td.host`, `history-list`, `select-all`, `load-more`, `column-picker`, `history-count`, `history-search`, `history-empty`, `uncommitted-marker`, `tabindex="0"` on exactly one row when it is rendered. The History testids are added to `ui/src/lib/testids.test.ts`'s preservation snapshot so a later refactor cannot drop one.

Fixtures in the existing specs have 1–6 rows, so all rows are in the window and the specs need no scrolling. Changes:
| spec | change | why |
|---|---|---|
| `perf-scroll.spec.ts` | rewritten driver and assertions (§3.3) | becomes the asserting gate; auto-load replaces timed Load-more clicks; reports `pages_loaded` from the network |
| `history-nav.spec.ts` | none expected; if the Tab walk or arrow tests need it, only the wait conditions change | roving model is preserved |
| `discoverability.spec.ts` | none expected; the "fits" test is the acceptance for the fixed column widths | fixed layout must satisfy it as is |
| `xss-corpus.spec.ts` | one new test (§3.3) | corpus rows reached by scrolling a virtualised list |
| new `history-virtual.spec.ts` | new | DOM bound, reveal-on-`j`, focus evacuation, selection outside the window, select-all count, live prepend anchor, auto-load + Load more, cap, `aria-rowcount`/`aria-rowindex`, row height constant |
| `mouse-free.spec.ts`, `tags-notes.spec.ts`, `history.spec.ts` | none | ≤ 2 rows |

A spec that must reach a row beyond the window scrolls `history-list` (`el.scrollTop = n * rowH` then waits for the row), never relies on Playwright's auto-scroll (it scrolls only to elements that exist).

### 3.2 Scoped keymap (D2)

#### 3.2.1 Scopes
```ts
export type Scope = 'global' | 'history' | 'repeater' | 'intercept' | 'viewer';
export const SCOPE_LABELS: Record<Scope, string> = {
  global: 'Everywhere', history: 'History', repeater: 'Repeater', intercept: 'Intercept', viewer: 'Message viewer',
};
// Where a scope can be active: the route scopes are mutually exclusive (one view
// is mounted); the viewer lives inside History (detail) and Repeater (response).
export const PARENTS: Record<Scope, readonly Scope[]> = {
  global: [], history: ['global'], repeater: ['global'], intercept: ['global'], viewer: ['history', 'repeater', 'global'],
};
export function coActive(a: Scope, b: Scope): boolean; // a === b || a ∈ PARENTS[b] || b ∈ PARENTS[a]
```
If Intercept ever embeds a `MessageViewer`, `PARENTS.viewer` gains `'intercept'`, which makes Find and Forward a conflict on `Ctrl+F`; that change must rebind one of them in the same commit (a unit test enumerates defaults and fails on any co-active clash, §3.2.4).

#### 3.2.2 Actions carry a scope
`ACTION_SCOPES: Record<Action, Scope>`; two new actions (13 in total):

| action | scope | default | registered by |
|---|---|---|---|
| `send-to-repeater` | global | `ctrl+r` | History, Intercept (whichever is mounted) |
| `toggle-intercept` | global | `ctrl+i` | Shell |
| `decode-selection` | global | `ctrl+shift+d` | Shell |
| `copy-as-curl` | global | `ctrl+shift+c` | the mounted copy menu (History detail, Repeater) |
| `copy-raw` | global | `ctrl+shift+s` | the mounted copy menu or Intercept |
| `show-shortcuts` | global | `?` | Shell |
| `focus-search` (new) | history | `/` | History |
| `send` | repeater | `ctrl+space` | Repeater |
| `tab-next` / `tab-prev` | repeater | `ctrl+shift+]` / `ctrl+shift+[` | Repeater |
| `forward` / `drop` | intercept | `ctrl+f` / `ctrl+d` | Intercept |
| `find` (new) | viewer | `ctrl+f` | each `MessageViewer`, bound to its root element |

"Global" means the chord is reserved app-wide; which view supplies the handler is unchanged from U2 (one handler per action, last registration wins, torn down on unmount). `send-to-repeater` and the copy actions stay global because more than one view handles them; splitting them per view would change nothing a user can observe.

`focus-search` replaces History's `<svelte:window onkeydown>` `/` handler, the last per-view window shortcut. History's row keys (`j`/`k`/arrows/`Enter`/`Space`/`Home`/`End`/`PageUp`/`PageDown`/`Esc`) stay list-widget keys handled by the list (ARIA grid convention, not remappable); the `?` sheet lists them in a fixed "History list" group.

#### 3.2.3 Dispatch: innermost focused scope first
- DOM marks scope roots with `data-keyscope`: History's root (`history-view`) `history`, Repeater's root `repeater`, Intercept's root `intercept`, each `MessageViewer` root `viewer`.
- `scopeChain(target, routeScope)`: the `data-keyscope` values of `target` and its ancestors, innermost first (deduplicated); then `routeScope` if not already present; then `global`. `routeScope` comes from the router (`routeScopeFor(path)`: `/` → history, `/repeater` → repeater, `/intercept` → intercept, else none) and covers focus outside the view (`<body>`, top bar, rail): Forward still fires from anywhere on the Intercept route, as in U2.
- `dispatch(e, routeScope)`:
  1. `chord = chordFromEvent(e)`; Meta → ignore (unchanged).
  2. A plain-key chord (no Ctrl/Alt) is ignored in an editable target (unchanged) and, new, inside `[role="menu"]` or `[role="dialog"]` (generalises History's old `/` rule; `?` inside an open menu no longer opens the sheet).
  3. For each scope in the chain: the action of that scope bound to `chord` (at most one per scope); its handler is the innermost element-bound handler whose element contains `target`, else the action's unbound handler. No handler → next scope.
  4. Call the handler. If it returns `false` it **declines**: no `preventDefault`, and dispatch stops (outer scopes do not get the key; the browser default runs). Otherwise `preventDefault()` and stop.
- Handler type becomes `(e: KeyboardEvent) => void | boolean`. `register(action, handler, opts?: { el?: Element })`: with `el`, the handler is element-bound (any number per action, chosen by containment); without, U2 semantics.
- Shell calls `keymap.dispatch(e, routeScopeFor(router.path))` and still suppresses dispatch while the `?` sheet is open.

#### 3.2.4 Find becomes a viewer action
- `MessageViewer` drops its local `keydown` listener and registers `find` bound to its root: on a textual tab (Pretty/Raw) it opens or refocuses the find bar; on Headers/Hex it returns `false` (declines), so the browser's own find runs and no outer `Ctrl+F` action (Forward, or a global action a user bound there) fires — exactly the U2 behaviour pinned by `settings.spec.ts` "Ctrl+F inside a viewer opens find and never reaches the global keymap".
- `VIEWER_FIND_CHORD` and the constant-based `findHint()` go; the hint is derived from the keymap (`"<chord> in the viewer"`, or no `kbd` when unbound).
- **Behaviour change (macOS):** U2's listener also caught `Cmd+F` in the viewer. The keymap never matches Meta (U2 invariant, kept), so `Cmd+F` now opens the browser's find; `Ctrl+F` opens the app's. Recorded in the CHANGELOG. (Open question for the maintainer, §7.)

#### 3.2.5 Conflicts per scope
- `validateChord(chord, action)` (interactive remap in Settings) refuses a chord held by another action **whose scope is co-active** with the action's scope; the message names the first such holder in `ACTIONS` order and its scope label: `Ctrl+R is already used by "Send to Repeater" (Everywhere).` The same chord in disjoint scopes is accepted (default example: Find in the viewer and Forward in Intercept, both `Ctrl+F`). Reserved and modifier-less rules unchanged.
- `remap()` steals only from co-active holders (U2 semantics otherwise). `reset()` cascades only over co-active holders.
- A unit test asserts the **defaults** have no co-active clash.

#### 3.2.6 Bindings may be unset; shadows are surfaced
- `bindings()` returns every action, in `ACTIONS` order, with `chord: string | null` and `scope`. An action whose chord was taken over shows **Not set** with its Change button in Settings (U2 dropped it from the table until reload, leaving no way to rebind it).
- `shadows(action): Action[]`: co-active actions on the same chord. Only reachable from loaded U2 data (§3.2.7); Settings shows a note on the inner action's row (`data-testid="shortcut-shadow-<action>"`, e.g. "Inside the message viewer this replaces Toggle intercept.") and the user resolves it by rebinding either.

#### 3.2.7 Persisted overrides migrate unchanged
- Storage key and format stay `localStorage["crosswire.keymap"] = {action: chord}`; nothing is rewritten on load (a V1 build only writes when the user remaps or resets).
- Loading applies the stored chords as U2 did. Same-scope clashes resolve with the U2 algorithm (the override whose chord another action ends up holding is dropped, order-independent). A co-active cross-scope clash between a stored override and a **new default** (`find` on `ctrl+f`, `focus-search` on `/`) is **kept** and resolved at dispatch by innermost-scope-wins, with the Settings shadow note. This reproduces U2 behaviour exactly for the case U2 pins (a U2 user who moved Toggle intercept to `Ctrl+F`: inside the viewer `Ctrl+F` still finds, elsewhere it still toggles). The one behaviour change is a U2 user who bound a global action to `/`: inside History `/` now focuses search (it was a no-op-or-race in U2, where History's window listener also claimed `/`).
- Unknown action names are dropped on load (unchanged), so a V1 `find` override read by an older build is ignored, not fatal.

#### 3.2.8 Hints derive live
- New `ui/src/lib/keymap-hints.svelte.ts`: a `$state` version bumped by `keymap.subscribe`, `chordLabel(action): string | undefined` and `findHint(): string | undefined` that read it, so every caller inside a template or `$derived` re-renders on remap. `copy-actions.svelte.ts`'s `chordLabel` is removed and its callers import from here.
- Callers that must read it reactively: `TopBar` (replace its private version counter), `SelectionToolbar`, `row-actions.ts` / `RowContextMenu`, `copy-actions.svelte.ts` `copyItems`, `ExchangeDetail`, `RepeaterTab`, `RepeaterResponse`, `Intercept`, `MessageViewer`, `ShortcutsDialog` (rows become `$derived`), `Settings`.
- Settings and the `?` sheet group shortcuts by scope in `SCOPE_LABELS` order (Everywhere, History, Repeater, Intercept, Message viewer); each group is a `<section data-testid="shortcut-group-<scope>">` with an `h3`; the row markup and testids (`shortcut-change-<action>`, `shortcut-reset-<action>`, `shortcut-error-<action>`, `shortcut-row` with `data-action`, `shortcuts-reset-all`) are unchanged. The sheet adds the fixed "History list" group (`data-testid="shortcut-fixed-row"`, not remappable).

### 3.3 Acceptance gates (D3)

#### 3.3.1 `perf-scroll.spec.ts` becomes an asserting gate
- **Driver:** real wheel events over `history-list` for 10 s as today; **no** Load-more clicks (auto-load pages). `pages_loaded` = count of `GET /api/v1/exchanges?…after=…` requests seen via `page.on('request')`.
- **Visible rows:** `ceil(list.clientHeight / rowH)` with `rowH` read from the first `history-row` after the scroll (outside the measured window). `OVERSCAN` is imported from `../src/lib/components/virtual-window` (pure TS, as the XSS spec already imports `src/lib` modules).
- **SUMMARY line** unchanged plus `visible_rows=` and `loaded_rows=`.
- **Assertions** when `CROSSWIRE_UI_PERF_ASSERT=1`:
  - `max_frame_ms ≤ 32` (the §13 wording "no frame > 32 ms"; first delta excluded as today),
  - `filter_ms < 300`,
  - `dom_rows ≤ 2 × visible_rows + OVERSCAN`,
  - `pages_loaded ≥ 1` and `loaded_rows ≥ 1,000` (the scroll really went deep; guards the R3 regression where `pages_loaded` was 0),
  - plus the existing gross floors (always on).
- `scripts/ui-perf.sh` defaults `CROSSWIRE_UI_PERF_ASSERT` to `1` when `ROWS ≤ 100000` and `0` above; an explicit value wins. So `just ui-perf` (100k) asserts, `CROSSWIRE_UI_PERF_ROWS=1000000 just ui-perf` and `just perf` stay report-only at 1M.
- **CI:** a new ubuntu step after `just first-capture-ui`: `just ui-perf` (100k, asserting). `playwright.perf-scroll.config.ts` gets `retries: process.env.CI ? 1 : 0`; a retry is a flake signal, not a pass to ignore (§6).
- `bench/README.md` §13 row and the "UI frame-time" section move from REPORT-ONLY to ASSERTED at 100k (report-only at 1M) with the new reference numbers; the justfile comments on `ui-perf` and `perf` say the same.

#### 3.3.2 Numbers that must hold
| run | max frame | filter | DOM rows |
|---|---|---|---|
| CI, 100k (asserted) | ≤ 32 ms | < 300 ms | ≤ 2 × visible + 8 |
| maintainer, 100k and 1M on the M2 reference machine (reported, must also meet) | ≤ 32 ms | < 300 ms | ≤ 2 × visible + 8 |

#### 3.3.3 Functional and security gates
- `history-virtual.spec.ts` (mocked API, 5,000 rows over 50 pages, plus a 50,100-row cap case served by a generator in the route handler): DOM bound at top, middle and bottom; `j` from the last rendered row reveals and focuses the next; wheel-scroll with a focused row keeps focus in the list and returns it when the row scrolls back; a checked row scrolled out and back is still checked and still counted; select-all counts all loaded rows; a `received` burst while scrolled to the middle leaves the anchored row at the same viewport offset (±1 px), and at the top shows the new rows; auto-load fires near the end and Load more still works with the keyboard; at the cap the button reads `Limit reached` and the cap note shows; `aria-rowcount` is `-1` while more pages exist and `n + 1` at the end; `aria-rowindex` of the first rendered row equals `start + 2`; every rendered row's height equals `rowH` ± 0.5 px.
- **XSS corpus over virtualised rows** (`xss-corpus.spec.ts`, new test): 2,000 rows where every 40th row carries a corpus payload in method, host, path and tags; scroll the list top to bottom in viewport steps (waiting one rAF each) and back; the existing `watch()` detector reports no dialog, off-origin request or navigation; every rendered payload is literal text; no element outside the table's own markup appears inside `tbody`; every row stays one line high. The custom properties written by the list are integers (`/^\d+(\.\d+)?px$/`), asserted.
- Keymap: unit tests (§4) and e2e: Ctrl+F in the viewer finds, in Intercept forwards (both with defaults); remapping Find to `Ctrl+Shift+F` in Settings updates the viewer button title without navigation; after remapping Send to Repeater in Settings, the History selection toolbar and row menu show the new chord on return, and the TopBar intercept switch title updates while Settings is still open; the U2 stored override `{forward:'ctrl+alt+f','toggle-intercept':'ctrl+f'}` keeps the `settings.spec.ts` find test green unchanged and shows the shadow note in Settings.

## 4. Testing summary
- **Vitest:** `virtual-window.test.ts` (window math at the edges, clamp on shrink, reveal above/below/visible, anchor round-trip across a prepend, auto-load predicate truth table); `history.test.ts` additions (in-flight guard, dedupe on append, cap on paging and prepend, `capped`, `generation`, coalesced events with an injected scheduler, `indexOf`); `keymap.test.ts` additions (scope table, `coActive`, defaults have no co-active clash, per-scope `validateChord`, element-bound handlers chosen by containment, decline stops dispatch, chain order viewer→history→global, route fallback, plain keys ignored in menus/dialogs, U2 storage fixtures load verbatim, shadows reported, `bindings()` lists unbound actions); `keymap-hints` reactive test under `$effect.root`; `testids.test.ts` History snapshot.
- **Playwright:** the specs in §3.1.12 and §3.3.3; the full default suite stays green.
- **Perf:** `just ui-perf` asserting at 100k; the maintainer runs 1M and records both rows in `bench/README.md`.

## 5. Constraints
- §11.1 unchanged: rows render text only; no `{@html}`, `innerHTML`, `srcdoc`, dynamic `href`/`src`, `<svelte:element>`; ESLint config unchanged and no new `eslint-disable`.
- CSP unchanged: no `style=` attribute or `style:` directive; the only style writes are `element.style.setProperty('--vt-before' | '--vt-after', '<n>px')` on `listEl`, values computed from indices.
- No new npm or Rust dependency; `package.json` and `package-lock.json` unchanged.
- No Rust, API, OpenAPI or store change.
- Bundle rebuilt with `just ui-build` and committed; `just ui-dist-check` clean; growth budget +6 KB JS / +1 KB CSS raw (the list logic replaces code; the keymap adds a scope table).
- All existing e2e test ids preserved.
- Conventional Commits, subject ≤ 72 chars, a body that says why; no AI attribution trailers.
- Version: the CHANGELOG entry goes under `[Unreleased]`; release-plz cuts 0.1.1. No manual version bump.

## 6. Risks
1. **A single dropped vsync fails the gate.** At 60 Hz one missed frame reads ~33.3 ms > 32. A GC pause, a JSON parse of a page, or CI runner noise can do it. Mitigations: auto-load parses 100 rows (small); `$state.raw` rows avoid proxy churn; the window update creates ≤ ~12 rows per frame; one CI retry. If CI flakes repeatedly, investigate the trace (a long task attribution) before touching the threshold; loosening the gate needs a maintainer ruling.
2. **Cold first filter query.** An earlier run saw ~450 ms on a cold first query at 100k; the current figure is 190 ms. The timed filter happens after the 10 s scroll (warm). If CI exceeds 300 ms, it is a store-side warm-up issue, out of scope for V1; the remedy is a maintainer ruling, not a harness change that hides it.
3. **Fixed column widths vs fonts.** `ch`/`rem` widths depend on the font stack; a platform with wider fallback fonts could clip a badge. The "fits" spec runs only on CI's Chromium. Widths keep ~10% headroom over the measured worst case.
4. **Fast scroll shows blank rows for a frame.** At ~12 rows per frame of wheel travel, an 8-row overscan can be outrun; the pad rows show the background briefly. Acceptable (no layout jump, no frame cost); a larger overscan costs DOM.
5. **Anchoring and sort ties.** Same-millisecond `created_at` ties may reorder between sort modes; the anchor is by id, so the viewport stays on the same row regardless.
6. **Scope shadows from old data** are rare (a U2 user who put a global action on `/` or `Ctrl+F`) and surfaced in Settings, not silently changed.
7. **Scrollbar semantics.** The scrollbar spans loaded rows only (up to 50,000); dragging it to the bottom auto-loads the next page, which grows the scroll height under the thumb. This is the existing keyset model made visible; a "virtual total" scrollbar would need a count query per filter and is out of scope.
8. **macOS Cmd+F** now reaches the browser's find inside the viewer (§3.2.4).

## 7. Open questions for the maintainer (defaults chosen; the plan proceeds with them)
1. **Cmd+F parity on macOS.** Default: dropped (the keymap never matches Meta). Alternative: a narrow, documented Meta-as-Ctrl alias for `viewer`-scoped actions on macOS only.
2. **Row cap of 50,000.** Default chosen from memory, sort latency and the Firefox element-height limit; the code cannot tell whether a tester ever needs more than 50,000 rows client-side without a server filter.
3. **CI cost of the asserting gate.** `just ui-perf` adds a release build of the server and the seed example plus seeding 100k rows (estimated 3–5 min on ubuntu runners). If that is too slow, the fallback is a nightly (`schedule`) job instead of every PR.
