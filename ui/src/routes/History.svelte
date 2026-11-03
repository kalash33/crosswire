<script lang="ts">
  import { onMount, tick, untrack } from 'svelte';
  import { SvelteSet } from 'svelte/reactivity';
  import { clientFilter, history, type ExchangeFilter, type Row } from '../lib/stores/history.svelte';
  import { events } from '../lib/stores/events.svelte';
  import ExchangeDetail from '../lib/components/ExchangeDetail.svelte';
  import SelectionToolbar from '../lib/components/SelectionToolbar.svelte';
  import RowContextMenu from '../lib/components/RowContextMenu.svelte';
  import { actionTarget, type RowActions } from '../lib/components/row-actions';
  import { scope } from '../lib/stores/scope.svelte';
  import { hostRule } from '../lib/components/scope-rows';
  import { keymap } from '../lib/keymap';
  import { repeater } from '../lib/stores/repeater.svelte';
  import { router } from '../lib/router.svelte';
  import { humanBytes, timeCell } from '../lib/format';
  import {
    historySplitDefault,
    TOGGLEABLE_COLUMNS,
    ariaRowCount,
    isTypingTarget,
    keyTarget,
    loadColumns,
    loadMoreLabel,
    saveColumns,
    type ColumnKey,
  } from '../lib/components/history-view';
  import {
    anchorAt,
    restoreAnchor,
    revealTop,
    shouldAutoLoad,
    windowFor,
    type Win,
  } from '../lib/components/virtual-window';
  import { buildIndex } from '../lib/stores/row-index';
  import Badge from '../lib/ui/Badge.svelte';
  import Button from '../lib/ui/Button.svelte';
  import Chip from '../lib/ui/Chip.svelte';
  import EmptyState from '../lib/ui/EmptyState.svelte';
  import IconButton from '../lib/ui/IconButton.svelte';
  import Input from '../lib/ui/Input.svelte';
  import Menu, { type MenuItem } from '../lib/ui/Menu.svelte';
  import Select from '../lib/ui/Select.svelte';
  import Splitter from '../lib/ui/Splitter.svelte';
  import { toasts } from '../lib/ui/toasts.svelte';
  import StartPanel from '../lib/components/StartPanel.svelte';
  import { showStart } from '../lib/components/start';
  import { connection } from '../lib/stores/connection.svelte';

  // ---- Filter inputs (applied on submit; the store builds the query) --------
  let fHost = $state('');
  let fHostContains = $state('');
  let fMethod = $state('');
  let fStatusMin = $state('');
  let fStatusMax = $state('');
  let fTag = $state('');
  let fState = $state('');
  let fScopeState = $state('');

  // The filter bar folds away behind the Filter button; open by default and
  // remembered per browser (storage may be unavailable: then it just stays open).
  const FILTERS_KEY = 'crosswire.history.filters-open';
  function loadFiltersOpen(): boolean {
    try {
      return localStorage.getItem(FILTERS_KEY) !== '0';
    } catch {
      return true;
    }
  }
  let filtersOpen = $state(loadFiltersOpen());
  function toggleFilters(): void {
    filtersOpen = !filtersOpen;
    try {
      localStorage.setItem(FILTERS_KEY, filtersOpen ? '1' : '0');
    } catch {
      // Not persisted; the toggle still applies for this page.
    }
  }

  function buildFilter(): ExchangeFilter {
    const f: ExchangeFilter = {};
    if (fHost.trim()) f.host = fHost.trim();
    if (fHostContains.trim()) f.host_contains = fHostContains.trim();
    if (fMethod.trim()) f.method = fMethod.trim();
    if (fStatusMin.trim()) f.status_min = Number(fStatusMin);
    if (fStatusMax.trim()) f.status_max = Number(fStatusMax);
    if (fTag.trim()) f.tag = fTag.trim();
    if (fState) f.state = fState;
    if (fScopeState) f.scope_state = fScopeState;
    return f;
  }

  // Number of filters in force (the applied filter, not the unsaved inputs).
  const activeFilters = $derived(
    Object.values(history.filter).filter((v) => v !== undefined && v !== '').length,
  );

  function applyFilters(e?: SubmitEvent): void {
    e?.preventDefault();
    selectedId = null;
    void history.applyFilter(buildFilter());
  }

  function clearFilters(): void {
    fHost = fHostContains = fMethod = fStatusMin = fStatusMax = fTag = '';
    fState = fScopeState = '';
    applyFilters();
  }

  // ---- Client-side sort over the loaded rows ---------------------------------
  type SortKey = 'created_at' | 'method' | 'host' | 'path' | 'status' | 'resp_len' | 'state';
  let sortKey = $state<SortKey>('created_at');
  let sortDir = $state<1 | -1>(-1); // newest first by default

  function sortBy(key: SortKey): void {
    if (sortKey === key) sortDir = sortDir === 1 ? -1 : 1;
    else {
      sortKey = key;
      sortDir = 1;
    }
  }

  function cmp(a: Row, b: Row): number {
    const av = a[sortKey];
    const bv = b[sortKey];
    if (av === bv) return 0;
    if (av === null || av === undefined) return 1;
    if (bv === null || bv === undefined) return -1;
    return (av < bv ? -1 : 1) * sortDir;
  }

  // The default order (Time, newest first) is the store's own order (server
  // `ORDER BY ulid DESC` plus newest-first live prepends), so it is used as is:
  // no copy, no sort pass over tens of thousands of rows per update.
  const defaultSort = $derived(sortKey === 'created_at' && sortDir === -1);
  const sortedRows = $derived(defaultSort ? history.rows : [...history.rows].sort(cmp));

  // ---- Client-side search over the loaded rows (combined with the server filter) ----
  let query = $state('');
  const searching = $derived(query.trim() !== '');
  // Every loaded row matching the search; the render window is a slice of it.
  const visibleRows = $derived(clientFilter(sortedRows, query));

  // id -> index in `visibleRows`. Unsorted and unsearched, that is the store's
  // own index; otherwise a map built once per `visibleRows` (lazily, on use).
  let indexFor: readonly Row[] | null = null;
  let indexMap: Map<string, number> = buildIndex([]);
  function vIndex(id: string | null): number {
    if (id === null) return -1;
    const rows = visibleRows;
    if (rows === history.rows) return history.indexOf(id);
    if (indexFor !== rows) {
      indexMap = buildIndex(rows);
      indexFor = rows;
    }
    return indexMap.get(id) ?? -1;
  }

  // First-run list/detail split, from the window width once at mount (a
  // stored ratio still wins): a laptop-width window gives the list more room.
  const splitDefault = historySplitDefault(typeof innerWidth === 'number' ? innerWidth : 0);

  // Digits of the largest `#` cell: sizes the # column (a class, not a style).
  const idxDigits = $derived(Math.min(5, String(visibleRows.length).length));

  // Footer note once the row cap stops paging (spec §3.1.7).
  const capNote = $derived(
    `${history.rows.length.toLocaleString('en-US')} rows loaded. Narrow the filter to see older exchanges.`,
  );

  // ---- Column picker (Host and Path are always shown) ------------------------
  let cols = $state<ColumnKey[]>(loadColumns());
  const shown = (k: ColumnKey): boolean => cols.includes(k);
  // 1 checkbox + # + Host + Path, plus the toggleable columns in force.
  const colCount = $derived(4 + cols.length);
  let colMenuAnchor = $state<HTMLElement | null>(null);

  function toggleColumn(key: ColumnKey): void {
    cols = shown(key) ? cols.filter((k) => k !== key) : TOGGLEABLE_COLUMNS.map((c) => c.key).filter((k) => k === key || shown(k));
    saveColumns(cols);
  }

  const columnItems = $derived<MenuItem[]>(
    TOGGLEABLE_COLUMNS.map((c) => ({
      label: c.label,
      icon: shown(c.key) ? ('check' as const) : undefined,
      testid: `column-${c.key}`,
      onselect: () => toggleColumn(c.key),
    })),
  );

  function ariaSort(key: SortKey): 'ascending' | 'descending' | 'none' {
    if (sortKey !== key) return 'none';
    return sortDir === 1 ? 'ascending' : 'descending';
  }

  // ---- Multi-select ----------------------------------------------------------
  const selected = new SvelteSet<string>();

  function toggleRow(id: string): void {
    if (selected.has(id)) selected.delete(id);
    else selected.add(id);
  }

  // A hidden row cannot be seen or unchecked, so a search (or the row cap
  // trimming it) drops its selection. A row merely outside the render window
  // stays selected.
  $effect(() => {
    void visibleRows;
    untrack(() => {
      for (const id of [...selected]) if (vIndex(id) < 0) selected.delete(id);
    });
  });

  function toggleAll(): void {
    if (checkedIds.length === visibleRows.length && visibleRows.length > 0) {
      for (const r of visibleRows) selected.delete(r.id);
    } else {
      for (const r of visibleRows) selected.add(r.id);
    }
  }

  // Checked ids in table order (the order bulk PATCHes are sent in).
  const checkedIds = $derived(visibleRows.filter((r) => selected.has(r.id)).map((r) => r.id));

  // ---- Detail selection ------------------------------------------------------
  let selectedId = $state<string | null>(null);
  // The row that currently holds keyboard focus (focusin bubbles from the row's
  // children). The row actions act on it, falling back to the opened row.
  let focusedId = $state<string | null>(null);
  let sendError = $state<string | null>(null);
  let sending = $state(false);
  // Bumped to re-create the detail after a bulk edit touched the open row.
  let detailEpoch = $state(0);
  let detail = $state<ReturnType<typeof ExchangeDetail>>();

  function openRow(id: string): void {
    selectedId = id;
    focusedId = id;
  }

  let rootEl = $state<HTMLElement>();
  let listEl = $state<HTMLElement>();

  // ---- The render window (spec §3.1.4) ---------------------------------------
  // Only `visibleRows.slice(win.start, win.end)` is in the DOM, between two
  // spacer rows whose heights are the `--vt-before`/`--vt-after` custom
  // properties on the list (CSSOM writes; CSP-clean, no style attribute).
  // Geometry is measured, never read on the scroll path: `rowH` from the first
  // rendered row, `headH`/`footH` from the sticky header/footer, `viewportH`
  // from the list; until the first measure `rowH` is the CSS token.
  const ROW_H_TOKEN = 26;
  let rowH = $state(ROW_H_TOKEN);
  let headH = $state(0);
  let footH = $state(0);
  let viewportH = $state(0);
  let measured = false;
  let win = $state.raw<Win>({ start: 0, end: 0, before: 0, after: 0 });
  const windowRows = $derived(visibleRows.slice(win.start, win.end));

  const nextFrame = (fn: () => void): void => {
    if (typeof requestAnimationFrame === 'function') requestAnimationFrame(() => fn());
    else setTimeout(fn, 16);
  };

  function windowAt(scrollTop: number): Win {
    return windowFor(scrollTop, viewportH, rowH, headH, visibleRows.length);
  }

  // Last pad heights written (and to which list), so an unchanged window
  // costs no style write.
  let padEl: HTMLElement | null = null;
  let padBefore = -1;
  let padAfter = -1;
  function applyWindow(w: Win): void {
    const cur = win;
    if (w.start !== cur.start || w.end !== cur.end || w.before !== cur.before || w.after !== cur.after) win = w;
    if (!listEl) return;
    if (padEl !== listEl) {
      padEl = listEl;
      padBefore = padAfter = -1;
    }
    if (w.before !== padBefore) {
      listEl.style.setProperty('--vt-before', `${w.before}px`);
      padBefore = w.before;
    }
    if (w.after !== padAfter) {
      listEl.style.setProperty('--vt-after', `${w.after}px`);
      padAfter = w.after;
    }
  }

  function measure(): void {
    const el = listEl;
    if (!el) return;
    viewportH = el.clientHeight;
    headH = el.querySelector('thead')?.getBoundingClientRect().height ?? 0;
    footH = el.querySelector('.foot')?.getBoundingClientRect().height ?? 0;
    // A hidden or not-yet-laid-out list measures 0: keep the token and try
    // again on the next ResizeObserver callback or rendered frame.
    const h = el.querySelector('tr[data-testid="history-row"]')?.getBoundingClientRect().height ?? 0;
    if (h > 0) {
      rowH = h;
      measured = true;
    }
  }

  /** Rows that fit between the sticky header and footer (PageUp/PageDown). */
  const pageRows = (): number => Math.max(1, Math.floor((viewportH - headH - footH) / rowH));

  /** The rendered element of row `id`, if it is in the window. */
  function rowEl(id: string | null): HTMLElement | null {
    if (!listEl || id === null) return null;
    for (const el of listEl.querySelectorAll<HTMLElement>('tr[data-testid="history-row"]')) {
      if (el.dataset.id === id) return el;
    }
    return null;
  }

  // ---- Focus (spec §3.1.8) ---------------------------------------------------
  // Focus is logical (by id) and the scroll follows it. When the focused row
  // is about to leave the window (wheel, scrollbar, a live prepend), focus is
  // parked on the list itself (never <body>) and `evacuated` remembers to put
  // it back when the row is rendered again.
  let evacuated = false;
  // True while the list is focused programmatically, so its focus handler
  // does not treat that as the user tabbing in.
  let parking = false;

  function park(): void {
    if (!listEl) return;
    parking = true;
    listEl.focus({ preventScroll: true });
    parking = false;
    evacuated = true;
  }

  /** Park focus if the focused row (or its checkbox) is outside window `w`. */
  function evacuateIfLeaving(w: Win): void {
    const el = listEl;
    const active = document.activeElement;
    if (!el || !(active instanceof HTMLElement) || active === el || !el.contains(active)) return;
    const row = active.closest<HTMLElement>('tr[data-testid="history-row"]');
    if (!row) return;
    const i = vIndex(row.dataset.id ?? null);
    if (i >= w.start && i < w.end) return;
    park();
  }

  const rovingId = $derived(
    vIndex(focusedId) >= 0 ? focusedId : vIndex(selectedId) >= 0 ? selectedId : (visibleRows[0]?.id ?? null),
  );
  const rovingIndex = $derived(vIndex(rovingId));
  // The list is the tab stop only while the roving row is outside the window
  // (set on the element: the scroll region has no interactive role of its own).
  const listTabIndex = $derived(rovingIndex >= 0 && (rovingIndex < win.start || rovingIndex >= win.end) ? 0 : -1);
  $effect(() => {
    if (listEl) listEl.tabIndex = listTabIndex;
  });

  /**
   * Move focus to row `i` of `visibleRows`: scroll it fully into view (window
   * computed synchronously from the new scrollTop), then focus it without a
   * second, native scroll. Focus is immediate when the row is already rendered.
   */
  function moveTo(i: number): void {
    const el = listEl;
    if (!el || i < 0 || i >= visibleRows.length) return;
    const id = visibleRows[i].id;
    focusedId = id;
    evacuated = false;
    const top = revealTop(i, el.scrollTop, viewportH || el.clientHeight, rowH, headH, footH);
    if (top !== null) {
      el.scrollTop = top;
      applyWindow(windowAt(el.scrollTop));
    }
    const now = rowEl(id);
    if (now) {
      now.focus({ preventScroll: true });
      return;
    }
    void tick().then(() => {
      if (focusedId === id) rowEl(id)?.focus({ preventScroll: true });
    });
  }

  // Esc in the search box or the detail pane lands here: the roving row, else the list.
  function focusList(): void {
    if (rovingId !== null) moveTo(rovingIndex);
    else listEl?.focus();
  }

  // Row keyboard model (§7.6/§11.1), one delegated handler on the list:
  // j/k and the arrows move one row, PageDown/PageUp a screenful, Home/End to
  // the first/last loaded row; Enter opens, Space toggles the checkbox. Only
  // a row, its checkbox or the list itself (focus parked) is a valid target,
  // so a key typed in any field or button never moves rows.
  function onListKey(e: KeyboardEvent): void {
    const el = listEl;
    if (!el || e.ctrlKey || e.altKey || e.metaKey) return;
    const t = e.target;
    let id: string | null;
    let onRow = false;
    if (t === el) {
      id = vIndex(focusedId) >= 0 ? focusedId : rovingId;
    } else {
      if (!(t instanceof HTMLElement)) return;
      const row = t.closest<HTMLElement>('tr[data-testid="history-row"]');
      if (!row || !el.contains(row)) return;
      onRow = t === row;
      const onBox = t instanceof HTMLInputElement && t.type === 'checkbox';
      if (!onRow && !onBox) return;
      id = row.dataset.id ?? null;
    }
    if (id === null) return;
    if (e.key === 'Enter') {
      e.preventDefault();
      openRow(id);
      if (t === el) moveTo(vIndex(id));
      return;
    }
    if (e.key === ' ' || e.key === 'Spacebar') {
      if (!onRow) return;
      e.preventDefault();
      toggleRow(id);
      return;
    }
    const to = keyTarget(e.key, vIndex(id), visibleRows.length, pageRows());
    if (to === null) return;
    e.preventDefault();
    moveTo(to);
  }

  function onListFocusIn(e: FocusEvent): void {
    const row = e.target instanceof HTMLElement ? e.target.closest<HTMLElement>('tr[data-testid="history-row"]') : null;
    if (row?.dataset.id) {
      focusedId = row.dataset.id;
      evacuated = false;
    } else if (e.target !== listEl) {
      // Focus moved on to other list chrome (select-all, a header, Load more):
      // the parked row no longer owns it.
      evacuated = false;
    }
  }

  // Focus leaving the list for good ends the parked state.
  function onListFocusOut(e: FocusEvent): void {
    const to = e.relatedTarget;
    if (!(to instanceof Node) || !listEl?.contains(to)) evacuated = false;
  }

  // Tabbing onto the list (its tab stop while the roving row is scrolled
  // away) reveals and focuses the roving row. A click on the list's chrome is
  // not a keyboard focus and leaves the scroll position alone.
  function onListFocus(e: FocusEvent): void {
    const el = listEl;
    if (!el || parking || e.target !== el || evacuated || rovingId === null) return;
    let keyboard = true;
    try {
      keyboard = el.matches(':focus-visible');
    } catch {
      // Selector unsupported: treat as keyboard focus.
    }
    if (keyboard) moveTo(rovingIndex);
  }

  // ---- Scroll (spec §3.1.5, §3.1.6) -----------------------------------------
  // One passive listener and at most one frame callback per frame: read
  // scrollTop (the only layout read), park focus if its row is leaving, move
  // the window when it changed, and fetch the next page near the loaded end.
  let framePending = false;
  // The scrollTop this component last wrote itself (a reset to the top, an
  // anchor restore). The scroll event it causes moves the window but is not
  // the user scrolling, so it never auto-loads (spec §3.1.6: no auto-load on
  // a filter apply, a search, a sort or a live event).
  let programmaticTop: number | null = null;
  function onScroll(): void {
    if (framePending) return;
    framePending = true;
    nextFrame(onFrame);
  }

  function onFrame(): void {
    framePending = false;
    const el = listEl;
    if (!el) return;
    const st = el.scrollTop;
    // Kept while frames still sit where it was written (two live flushes in
    // successive frames deliver their scroll events a frame late); the first
    // frame anywhere else is the user moving, and clears it.
    const ours = programmaticTop !== null && Math.abs(st - programmaticTop) <= 1;
    if (!ours) programmaticTop = null;
    const w = windowAt(st);
    evacuateIfLeaving(w);
    applyWindow(w);
    // A failed page is retried with Load more, not by every scroll frame.
    if (ours || history.error) return;
    const s = { hasMore: history.hasMore, loading: history.loading, searching, capped: history.capped };
    if (shouldAutoLoad(w, visibleRows.length, s)) void history.loadMore();
  }

  $effect(() => {
    const el = listEl;
    if (!el) return;
    measured = false;
    let pending = false;
    const ro =
      typeof ResizeObserver === 'function'
        ? new ResizeObserver(() => {
            if (pending) return;
            pending = true;
            nextFrame(() => {
              pending = false;
              measure();
            });
          })
        : null;
    ro?.observe(el);
    el.addEventListener('scroll', onScroll, { passive: true });
    el.addEventListener('keydown', onListKey);
    el.addEventListener('focus', onListFocus);
    el.addEventListener('focusin', onListFocusIn);
    el.addEventListener('focusout', onListFocusOut);
    el.addEventListener('contextmenu', onListContextMenu);
    return () => {
      ro?.disconnect();
      el.removeEventListener('scroll', onScroll);
      el.removeEventListener('keydown', onListKey);
      el.removeEventListener('focus', onListFocus);
      el.removeEventListener('focusin', onListFocusIn);
      el.removeEventListener('focusout', onListFocusOut);
      el.removeEventListener('contextmenu', onListContextMenu);
    };
  });

  // First rows on screen: measure the real row height once (and again on the
  // next frame while it still reads 0).
  $effect(() => {
    if (windowRows.length === 0 || measured) return;
    nextFrame(measure);
  });

  // ---- Window maintenance on data and geometry changes (spec §3.1.9, §3.1.10) ----
  // Runs before the DOM update whenever the rows, the geometry or the view
  // (filter generation, search, sort) change:
  //  - a replace, a search or a sort starts again at the top;
  //  - otherwise (live prepend, committed update, page append, tag patch) the
  //    first fully visible row keeps its viewport offset, found by id; at the
  //    top (scrollTop < rowH/2) the list follows the head and new rows show.
  let seenRows: readonly Row[] | null = null;
  let seenGen = -1;
  let seenView = '';
  let pendingTop: number | null = null;

  /** The anchor (first fully visible row of `rows`) at `scrollTop`, O(1). */
  function anchorIn(rows: readonly Row[], scrollTop: number): { id: string; offset: number } | null {
    if (!(rowH > 0)) return null;
    const i0 = Math.max(0, Math.floor((scrollTop - headH) / rowH));
    const ids = rows.slice(i0, i0 + 2).map((r) => r.id);
    return anchorAt(scrollTop - i0 * rowH, rowH, headH, ids);
  }

  $effect.pre(() => {
    const rows = visibleRows;
    const gen = history.generation;
    const view = `${query}\u0000${sortKey}\u0000${sortDir}`;
    const el = listEl;
    void rowH;
    void headH;
    void viewportH;
    untrack(() => {
      const prev = seenRows;
      const reset = gen !== seenGen || view !== seenView;
      seenRows = rows;
      seenGen = gen;
      seenView = view;
      const st = el ? el.scrollTop : 0;
      let top = st;
      if (reset) {
        top = 0;
      } else if (prev !== null && prev !== rows && st >= rowH / 2) {
        const a = anchorIn(prev, st);
        const t = a ? restoreAnchor(a, vIndex, rowH, headH) : null;
        if (t !== null) top = Math.max(0, t);
      }
      const w = windowAt(top);
      evacuateIfLeaving(w);
      applyWindow(w);
      if (el && top !== st) {
        programmaticTop = top;
        el.scrollTop = top;
        // The pads already have their new heights; re-apply once the rows are
        // in the DOM in case the browser clamped against the old height.
        pendingTop = top;
        void tick().then(() => {
          if (listEl && pendingTop !== null && Math.abs(listEl.scrollTop - pendingTop) > 1) {
            programmaticTop = pendingTop;
            listEl.scrollTop = pendingTop;
          }
          pendingTop = null;
        });
      }
    });
  });

  // A parked focus returns to its row as soon as the row is rendered again.
  $effect(() => {
    void windowRows;
    untrack(() => {
      if (!evacuated || !listEl || document.activeElement !== listEl) return;
      const i = vIndex(focusedId);
      if (i < win.start || i >= win.end) return;
      const el = rowEl(focusedId);
      if (!el) return;
      el.focus({ preventScroll: true });
      evacuated = false;
    });
  });

  // `/` focuses the search box unless the user is typing in a field or working
  // in a menu/dialog. (The Shell's keymap does not bind `/`, so it falls through.)
  function onWindowKey(e: KeyboardEvent): void {
    if (e.key !== '/' || e.ctrlKey || e.altKey || e.metaKey || e.defaultPrevented) return;
    if (isTypingTarget(e.target)) return;
    if (e.target instanceof Element && e.target.closest('[role="menu"], [role="dialog"]')) return;
    const box = rootEl?.querySelector<HTMLInputElement>('[data-testid="history-search"]');
    if (!box) return;
    e.preventDefault();
    box.focus();
    box.select();
  }

  function onSearchKey(e: KeyboardEvent): void {
    if (e.key !== 'Escape') return;
    e.preventDefault();
    query = '';
    // The cleared query re-renders the full list first; then focus its row.
    void tick().then(focusList);
  }

  function onDetailKey(e: KeyboardEvent): void {
    // Escape inside a field (tag input, notes, CodeMirror) keeps the field's own meaning.
    if (e.key !== 'Escape' || e.defaultPrevented || isTypingTarget(e.target)) return;
    e.preventDefault();
    focusList();
  }

  const rowById = (id: string | null): Row | null => (id ? history.byId(id) : null);
  const selectedRow = $derived(rowById(selectedId));
  // One action target for the toolbar AND the Ctrl+R chord: the focused row,
  // then the opened row. The toolbar names it when it is not the open row.
  const target = $derived(actionTarget(focusedId, selectedId, visibleRows));
  const targetRow = $derived(rowById(target.id));

  // ---- Row actions (selection toolbar + context menu) ------------------------
  function hostInScope(host: string): boolean {
    const want = hostRule(host);
    return scope.scope.include.some(
      (r) => r.host.type === want.host.type && r.host.value === want.host.value && JSON.stringify(r.ports) === JSON.stringify(want.ports),
    );
  }

  // Add include rules for hosts not already there (needs the admin cap; a
  // failure surfaces the server message in `scope.error` and a toast).
  async function addHosts(hosts: string[]): Promise<void> {
    if (hosts.length === 0) {
      toasts.push({ kind: 'info', text: 'No host to add.' });
      return;
    }
    if (!scope.loaded) await scope.load();
    const fresh = hosts.filter((h) => !hostInScope(h));
    if (fresh.length === 0) {
      toasts.push({ kind: 'info', text: hosts.length === 1 ? `${hosts[0]} is already in scope.` : 'All hosts are already in scope.' });
      return;
    }
    const ok = await scope.save({
      include: [...scope.scope.include, ...fresh.map(hostRule)],
      exclude: scope.scope.exclude,
    });
    if (ok) {
      toasts.push({
        kind: 'success',
        text: fresh.length === 1 ? `Added ${fresh[0]} to scope.` : `Added ${fresh.length} hosts to scope.`,
      });
    } else {
      toasts.push({ kind: 'error', text: `Scope not saved: ${scope.error ?? 'unknown error'}` });
    }
  }

  // Tag: open the row, then focus the detail's tag input. The detail loads
  // asynchronously, so retry briefly until the input exists.
  async function tagRow(id: string): Promise<void> {
    openRow(id);
    await tick();
    for (let i = 0; i < 40; i++) {
      const el = document.querySelector<HTMLInputElement>('[data-testid="tag-input"]');
      if (el && selectedId === id) {
        el.focus();
        return;
      }
      await new Promise((r) => setTimeout(r, 50));
    }
  }

  async function sendToRepeater(id: string): Promise<void> {
    sendError = null;
    sending = true;
    try {
      await repeater.sendFromExchange(id);
      router.navigate('/repeater');
    } catch (e) {
      sendError = e instanceof Error ? e.message : String(e);
      toasts.push({ kind: 'error', text: `Send to Repeater failed: ${sendError}` });
    } finally {
      sending = false;
    }
  }

  async function previewRow(id: string): Promise<void> {
    openRow(id);
    await tick();
    await detail?.openPreview();
  }

  function actionsFor(id: string): RowActions {
    return {
      open: () => openRow(id),
      send: () => void sendToRepeater(id),
      tag: () => void tagRow(id),
      addHost: () => {
        const host = rowById(id)?.host;
        if (host) void addHosts([host]);
      },
      preview: () => void previewRow(id),
    };
  }

  // ---- Context menu ----------------------------------------------------------
  let menuId = $state<string | null>(null);
  let menuAnchor = $state<{ x: number; y: number } | null>(null);
  let menuRowEl: HTMLElement | null = null;

  // Delegated on the list: a right-click or the context-menu key on a row.
  function onListContextMenu(e: MouseEvent): void {
    const row = e.target instanceof Element ? e.target.closest<HTMLElement>('tr[data-testid="history-row"]') : null;
    const id = row?.dataset.id;
    if (!row || !id) return;
    e.preventDefault();
    menuRowEl = row;
    // The context-menu key reports (0,0): place the menu at the row instead.
    if (e.clientX === 0 && e.clientY === 0) {
      const r = row.getBoundingClientRect();
      menuAnchor = { x: r.left + 24, y: r.bottom };
    } else {
      menuAnchor = { x: e.clientX, y: e.clientY };
    }
    menuId = id;
  }

  // Back to the row the menu was opened on, if the window still renders it;
  // otherwise focus parks on the list until the row scrolls back in.
  function closeMenu(): void {
    menuId = null;
    if (menuRowEl?.isConnected) menuRowEl.focus({ preventScroll: true });
    else park();
    menuRowEl = null;
  }

  // ---- Bulk actions ----------------------------------------------------------
  let bulkTagValue = $state('');
  let bulkBusy = $state(false);

  async function bulkTag(e?: SubmitEvent): Promise<void> {
    e?.preventDefault();
    const ids = checkedIds;
    if (ids.length === 0 || bulkBusy) return;
    bulkBusy = true;
    try {
      const res = await history.bulkTag(ids, bulkTagValue);
      const tag = bulkTagValue.trim();
      if (res.failed.length === 0) {
        toasts.push({ kind: 'success', text: `Tagged ${res.ok.length} exchange${res.ok.length === 1 ? '' : 's'} "${tag}".` });
        bulkTagValue = '';
      } else {
        toasts.push({
          kind: 'error',
          text: `Tagged ${res.ok.length}, failed ${res.failed.length}: ${res.failed[0].error}`,
        });
      }
      if (selectedId && ids.includes(selectedId)) detailEpoch++;
    } catch (err) {
      toasts.push({ kind: 'error', text: `Tag not applied: ${err instanceof Error ? err.message : String(err)}` });
    } finally {
      bulkBusy = false;
    }
  }

  function bulkAddHosts(): void {
    void addHosts(history.bulkAddHosts(checkedIds));
  }

  // send-to-repeater (Ctrl+R): seed a repeater tab from the focused/opened row
  // and navigate there. Registered while History is mounted; torn down on
  // unmount so the chord is owned by whichever view is live.
  // Start panel (U1 D3): first-run guidance instead of an empty table, only
  // once the first page has loaded, with no error and no filter applied. A
  // `received` event adds a row, which hides it.
  let firstLoadDone = $state(false);
  const start = $derived(
    showStart(sortedRows.length, activeFilters > 0, history.loading || !firstLoadDone || !!history.error),
  );

  onMount(() => {
    events.start();
    void connection.load();
    void history.applyFilter({}).finally(() => (firstLoadDone = true));
    const off = keymap.register('send-to-repeater', () => {
      if (target.id) void sendToRepeater(target.id);
    });
    return off;
  });

  /** `timeCell` split into its date part (older rows only) and its clock. */
  function timeParts(ms: number): { date: string; clock: string } {
    const t = timeCell(ms);
    const sp = t.lastIndexOf(' ');
    return sp > 0 ? { date: t.slice(0, sp), clock: t.slice(sp) } : { date: '', clock: t };
  }

  function fullTime(ms: number): string {
    try {
      return new Date(ms).toLocaleString();
    } catch {
      return String(ms);
    }
  }
</script>

{#snippet sortHead(key: SortKey, label: string, cls = '')}
  <th class={cls} aria-sort={ariaSort(key)}>
    <button type="button" onclick={() => sortBy(key)}
      >{label}<span class="arrow" aria-hidden="true">{sortKey === key ? (sortDir === 1 ? '▲' : '▼') : ''}</span></button
    >
  </th>
{/snippet}

<svelte:window onkeydown={onWindowKey} />

<div class="history" data-testid="history-view" bind:this={rootEl}>
  {#if !start}
  <div class="bar">
    <Button
      size="sm"
      icon="filter"
      data-testid="filter-toggle"
      aria-expanded={filtersOpen}
      aria-controls="history-filters"
      onclick={toggleFilters}
      >Filter{#if activeFilters > 0}<span class="count-badge" data-testid="filter-count"
          ><Badge kind="neutral" value={activeFilters} /></span
        >{/if}</Button
    >
    {#if activeFilters > 0 && !filtersOpen}
      <Button size="sm" variant="ghost" data-testid="filter-clear-quick" onclick={clearFilters}>Clear filters</Button>
    {/if}

    <Input
      size="sm"
      class="search"
      type="search"
      data-testid="history-search"
      placeholder="Search host, path, tags  /"
      aria-label="Search loaded exchanges by host, path or tag"
      autocomplete="off"
      spellcheck={false}
      bind:value={query}
      onkeydown={onSearchKey}
    />
    <IconButton
      size="sm"
      icon="settings"
      label="Columns"
      data-testid="column-picker"
      aria-haspopup="menu"
      aria-expanded={colMenuAnchor !== null}
      onclick={(e) => (colMenuAnchor = colMenuAnchor ? null : e.currentTarget)}
    />

    {#if checkedIds.length > 0}
      <form class="bulk" data-testid="bulk-bar" aria-label="Bulk actions" onsubmit={bulkTag}>
        <span class="bulk-count">{checkedIds.length} selected</span>
        <Input
          size="sm"
          placeholder="Tag selected…"
          aria-label="Tag to add to the selected exchanges"
          data-testid="bulk-tag-input"
          bind:value={bulkTagValue}
        />
        <Button
          size="sm"
          type="submit"
          icon="tag"
          data-testid="bulk-tag-apply"
          disabled={bulkBusy || bulkTagValue.trim() === ''}>Apply</Button
        >
        <Button size="sm" icon="scope" data-testid="bulk-add-hosts" onclick={bulkAddHosts}>Add hosts to scope</Button>
      </form>
    {/if}
  </div>

  {#if filtersOpen}
    <form class="filters" id="history-filters" data-testid="history-filters" onsubmit={applyFilters}>
      <Input size="sm" mono data-testid="filter-host" placeholder="host (exact)" aria-label="host (exact)" bind:value={fHost} />
      <Input
        size="sm"
        mono
        data-testid="filter-host-contains"
        placeholder="host contains"
        aria-label="host contains"
        bind:value={fHostContains}
      />
      <Input size="sm" class="w-method" data-testid="filter-method" placeholder="method" aria-label="method" bind:value={fMethod} />
      <Input
        size="sm"
        class="w-status"
        data-testid="filter-status-min"
        type="number"
        placeholder="status ≥"
        aria-label="status at least"
        bind:value={fStatusMin}
      />
      <Input
        size="sm"
        class="w-status"
        data-testid="filter-status-max"
        type="number"
        placeholder="status ≤"
        aria-label="status at most"
        bind:value={fStatusMax}
      />
      <Input size="sm" data-testid="filter-tag" placeholder="tag" aria-label="tag" bind:value={fTag} />
      <Select size="sm" data-testid="filter-state" bind:value={fState} aria-label="recording state">
        <option value="">state: any</option>
        <option value="received">received</option>
        <option value="committed">committed</option>
        <option value="error">error</option>
      </Select>
      <Select size="sm" data-testid="filter-scope-state" bind:value={fScopeState} aria-label="scope state">
        <option value="">scope: any</option>
        <option value="in">in</option>
        <option value="out">out</option>
        <option value="empty">empty</option>
        <option value="host-mismatch">host-mismatch</option>
      </Select>
      <Button size="sm" variant="primary" type="submit" data-testid="filter-apply">Apply</Button>
      <Button size="sm" variant="ghost" data-testid="filter-clear" onclick={clearFilters}>Clear</Button>
    </form>
  {/if}
  {/if}

  {#if history.error}
    <p class="error" data-testid="history-error" role="alert">{history.error}</p>
  {/if}
  {#if sendError}
    <p class="error" data-testid="history-send-error" role="alert">{sendError}</p>
  {/if}
  {#if scope.error}<p class="error" role="alert" data-testid="scope-action-error">{scope.error}</p>{/if}

  {#if start}
    <StartPanel
      proxyAddr={connection.proxyAddrOrDefault}
      proxyReported={connection.proxyAddr !== null}
      apiOrigin={connection.apiOrigin}
    />
  {:else}
  <div class="layout">
    <Splitter direction="row" storageKey="history-detail" initial={splitDefault} min={[320, 360]} label="Resize history and detail">
      {#snippet a()}
        <div class="list" data-testid="history-list" tabindex="-1" bind:this={listEl}>
          <table
            class={`d${idxDigits}`}
            class:no-time={!shown('time')}
            class:no-method={!shown('method')}
            class:no-status={!shown('status')}
            class:no-len={!shown('len')}
            class:no-state={!shown('state')}
            class:no-tags={!shown('tags')}
            aria-rowcount={ariaRowCount(visibleRows.length, history.hasMore, history.capped)}
          >
            <colgroup>
              <col class="c-sel" />
              <col class="c-idx" />
              {#if shown('time')}<col class="c-time" />{/if}
              {#if shown('method')}<col class="c-method" />{/if}
              <col class="c-host" />
              <col class="c-path" />
              {#if shown('status')}<col class="c-status" />{/if}
              {#if shown('len')}<col class="c-len" />{/if}
              {#if shown('state')}<col class="c-state" />{/if}
              {#if shown('tags')}<col class="c-tags" />{/if}
            </colgroup>
            <thead>
              <tr aria-rowindex="1">
                <th class="sel">
                  <input
                    type="checkbox"
                    aria-label="select all"
                    data-testid="select-all"
                    checked={visibleRows.length > 0 && checkedIds.length === visibleRows.length}
                    onchange={toggleAll}
                  />
                </th>
                <th class="num">#</th>
                {#if shown('time')}{@render sortHead('created_at', 'Time')}{/if}
                {#if shown('method')}{@render sortHead('method', 'Method')}{/if}
                {@render sortHead('host', 'Host')}
                {@render sortHead('path', 'Path')}
                {#if shown('status')}{@render sortHead('status', 'Status')}{/if}
                {#if shown('len')}{@render sortHead('resp_len', 'Length', 'num')}{/if}
                {#if shown('state')}{@render sortHead('state', 'State')}{/if}
                {#if shown('tags')}<th>Tags</th>{/if}
              </tr>
            </thead>
            <tbody>
              <!-- Spacer rows stand in for the rows above and below the window;
                   their heights are the list's --vt-before/--vt-after. -->
              <tr class="pad" aria-hidden="true" data-testid="history-pad-top"><td class="pad-top" colspan={colCount}></td></tr>
              {#each windowRows as row, j (row.id)}
                <!-- Keyboard-operable row (§7.6): focusable (roving tabindex),
                     keys, focus and the context menu are delegated to the list. -->
                <tr
                  class="row"
                  class:uncommitted={row.uncommitted}
                  class:selected={selectedId === row.id}
                  class:checked={selected.has(row.id)}
                  data-testid="history-row"
                  data-id={row.id}
                  data-uncommitted={row.uncommitted}
                  aria-rowindex={win.start + j + 2}
                  tabindex={row.id === rovingId ? 0 : -1}
                  aria-label={`exchange ${row.method ?? ''} ${row.host ?? ''}${row.path ?? ''}`}
                >
                  <td class="sel">
                    <input
                      type="checkbox"
                      aria-label={`select ${row.id}`}
                      checked={selected.has(row.id)}
                      onchange={() => toggleRow(row.id)}
                    />
                  </td>
                  <td class="num idx">{win.start + j + 1}</td>
                  {#if shown('time')}
                    {@const t = timeParts(row.created_at)}
                    <td class="open time" title={fullTime(row.created_at)} onclick={() => openRow(row.id)}>
                      {#if row.uncommitted}<span class="dot" title="Uncommitted (not yet durable)" data-testid="uncommitted-marker"
                          >●</span
                        >{/if}{#if t.date}<span class="date">{t.date}</span>{/if}<span class="clock">{t.clock}</span>
                    </td>
                  {/if}
                  {#if shown('method')}
                    <td class="open" onclick={() => openRow(row.id)}>
                      {#if row.method}<Badge kind="method" value={row.method} />{/if}
                    </td>
                  {/if}
                  <td class="open host" title={row.host ?? ''} onclick={() => openRow(row.id)}
                    >{#if row.uncommitted && !shown('time')}<span
                        class="dot"
                        title="Uncommitted (not yet durable)"
                        data-testid="uncommitted-marker">●</span
                      >{/if}{row.host ?? ''}</td
                  >
                  <td class="open path" title={row.path ?? ''} onclick={() => openRow(row.id)}>{row.path ?? ''}</td>
                  {#if shown('status')}
                    <td class="open" onclick={() => openRow(row.id)}>
                      {#if row.status !== null}<Badge kind="status" value={row.status} />{/if}
                    </td>
                  {/if}
                  {#if shown('len')}
                    <td
                      class="open num"
                      title={row.resp_len === null ? '' : `${row.resp_len} bytes`}
                      onclick={() => openRow(row.id)}>{row.resp_len === null ? '' : humanBytes(row.resp_len)}</td
                    >
                  {/if}
                  {#if shown('state')}
                    <td class="open" onclick={() => openRow(row.id)}><Badge kind="state" value={row.state} /></td>
                  {/if}
                  {#if shown('tags')}
                    <td class="tags" title={row.tags.join(', ')}>
                      <span class="chips"
                        >{#each row.tags as tag (tag)}<Chip>{tag}</Chip>{/each}{#if row.tags.length > 1}<span class="more"
                            >+{row.tags.length - 1}</span
                          >{/if}</span
                      >
                    </td>
                  {/if}
                </tr>
              {/each}
              {#if visibleRows.length === 0 && !history.loading}
                <tr>
                  <td colspan={colCount} class="empty" data-testid="history-empty">
                    {searching
                      ? 'No loaded exchange matches the search.'
                      : activeFilters > 0
                        ? 'No exchanges match these filters.'
                        : 'No exchanges.'}
                  </td>
                </tr>
              {/if}
              <tr class="pad" aria-hidden="true" data-testid="history-pad-bottom"><td class="pad-bottom" colspan={colCount}></td></tr>
            </tbody>
          </table>

          <div class="foot">
            <span class="count" data-testid="history-count">{searching
                ? `${visibleRows.length} of ${sortedRows.length} loaded match`
                : `${sortedRows.length} loaded`} · {checkedIds.length} selected</span>
            {#if history.capped && history.hasMore}
              <span class="cap-note" data-testid="history-cap-note" title={capNote}>{capNote}</span>
            {/if}
            <Button
              size="sm"
              data-testid="load-more"
              disabled={!history.hasMore || history.loading || history.capped}
              onclick={() => void history.loadMore()}
            >
              {loadMoreLabel({ loading: history.loading, hasMore: history.hasMore, capped: history.capped })}
            </Button>
          </div>
        </div>
      {/snippet}
      {#snippet b()}
        <aside class="detail-pane" data-testid="history-detail">
          {#if target.id}
            <SelectionToolbar
              id={target.id}
              row={targetRow}
              actions={actionsFor(target.id)}
              busy={sending}
              showTarget={target.differs}
              index={target.index}
            />
          {/if}
          <!-- Focus entering the open exchange makes it the action target again. -->
          <div class="detail-body" role="presentation" onfocusin={() => (focusedId = selectedId)} onkeydown={onDetailKey}>
            {#if selectedId}
              {#key `${selectedId}-${detailEpoch}`}
                <ExchangeDetail bind:this={detail} id={selectedId} row={selectedRow} />
              {/key}
            {:else}
              <EmptyState
                icon="history"
                title="Open an exchange"
                body="Click a row or press Enter on it to see the request and response. Right-click a row for its actions."
              />
            {/if}
          </div>
        </aside>
      {/snippet}
    </Splitter>
  </div>
  {/if}

  {#if colMenuAnchor}
    <Menu
      items={columnItems}
      open={colMenuAnchor !== null}
      anchor={colMenuAnchor}
      onclose={() => (colMenuAnchor = null)}
      label="Columns"
      testid="column-menu"
    />
  {/if}

  {#if menuId}
    <RowContextMenu
      id={menuId}
      row={rowById(menuId)}
      open={menuId !== null}
      anchor={menuAnchor}
      actions={actionsFor(menuId)}
      onclose={closeMenu}
    />
  {/if}
</div>

<style>
  .history {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
    height: 100%;
    min-width: 0;
    min-height: 0;
  }
  .bar {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--s-2);
    min-width: 0;
  }
  .count-badge {
    margin-left: var(--s-1);
    display: inline-flex;
  }
  .bulk {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    margin-left: auto;
    padding: 2px var(--s-2);
    border: 1px solid var(--accent-soft);
    border-radius: var(--r-2);
    background: var(--selection);
  }
  .bulk-count {
    font-size: var(--fs-sm);
    font-weight: 600;
    color: var(--fg);
    white-space: nowrap;
  }
  .filters {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-2);
    align-items: center;
    padding: var(--s-2);
    border: 1px solid var(--border);
    border-radius: var(--r-2);
    background: var(--bg-elev);
  }
  .filters :global(.w-status) {
    width: 6rem;
  }
  .filters :global(.w-method) {
    width: 6rem;
  }
  .layout {
    flex: 1 1 auto;
    min-height: 0;
    border: 1px solid var(--border);
    border-radius: var(--r-2);
    background: var(--bg-elev);
    overflow: hidden;
  }
  .list {
    /* Declared row height; the authoritative value is measured (spec §3.1.2). */
    --history-row-h: 26px;
    height: 100%;
    min-width: 0;
    overflow: auto;
    /* The list anchors its own scroll position by row id; the browser's
       scroll anchoring would adjust it a second time on every window move. */
    overflow-anchor: none;
    container-type: inline-size;
  }
  /* A narrow list (the default split on a laptop) gives Host and Path room:
     older rows show only their date (the full time stays in `title`), and the
     tag chips ellipsize sooner. */
  @container (max-width: 720px) {
    td.time .date + .clock {
      display: none;
    }
    td.tags .chips {
      max-width: calc(3.75rem - 8px);
    }
    /* One chip and a "+N" count; the full list stays in the cell's `title`. */
    .chips :global(.chip:nth-child(n + 2)) {
      display: none;
    }
    .chips .more {
      display: inline;
    }
  }
  /* Fixed layout: the column widths come from the <col> elements, never from
     whichever rows the window happens to render (auto layout would make every
     column jump as an OPTIONS badge or a long date scrolls in, and measure the
     whole table to do it). Host takes 40% of what the fixed columns leave and
     Path the rest, both ellipsized with the full text in `title`, so the table
     fits the default 45/55 split without a horizontal scroll. A hidden
     column's width is zeroed so Host and Path share its room. */
  table {
    /* Each fits its worst case (the cap's `50000`, a date and clock with the
       uncommitted dot, OPTIONS, `committed`, a header with its sort arrow). */
    --w-sel: 1.375rem;
    --w-idx: calc(5ch + 8px);
    --w-time: calc(15ch + 20px);
    --w-method: 4rem;
    --w-status: 3.5rem;
    --w-len: 3.75rem;
    --w-state: 4.75rem;
    --w-tags: 7rem;
    width: 100%;
    table-layout: fixed;
    border-collapse: separate;
    border-spacing: 0;
    font-size: var(--fs-sm);
  }
  /* The # column is as wide as the loaded total's digits (up to the 50,000 cap). */
  table.d1,
  table.d2 {
    --w-idx: calc(2ch + 8px);
  }
  table.d3 {
    --w-idx: calc(3ch + 8px);
  }
  table.d4 {
    --w-idx: calc(4ch + 8px);
  }
  /* Narrow list (after the base rule so it wins): the time cell shows a date
     or a clock, and the tag strip one chip plus "+N". */
  @container (max-width: 720px) {
    table {
      --w-time: calc(8ch + 16px);
      --w-tags: 3.75rem;
    }
  }
  table.no-time {
    --w-time: 0px;
  }
  table.no-method {
    --w-method: 0px;
  }
  table.no-status {
    --w-status: 0px;
  }
  table.no-len {
    --w-len: 0px;
  }
  table.no-state {
    --w-state: 0px;
  }
  table.no-tags {
    --w-tags: 0px;
  }
  .c-sel {
    width: var(--w-sel);
  }
  .c-idx {
    width: var(--w-idx);
  }
  .c-time {
    width: var(--w-time);
  }
  .c-method {
    width: var(--w-method);
  }
  .c-status {
    width: var(--w-status);
  }
  .c-len {
    width: var(--w-len);
  }
  .c-state {
    width: var(--w-state);
  }
  .c-tags {
    width: var(--w-tags);
  }
  .c-host {
    width: max(
      2.75rem,
      calc(
        (100cqw - var(--w-sel) - var(--w-idx) - var(--w-time) - var(--w-method) - var(--w-status) - var(--w-len) -
            var(--w-state) - var(--w-tags)) * 0.4
      )
    );
  }
  /* .c-path: no width, it takes the rest. */
  thead th {
    position: sticky;
    top: 0;
    z-index: 1;
    background: var(--bg-elev);
    font-weight: 600;
    color: var(--fg-muted);
  }
  th,
  td {
    text-align: left;
    padding: 4px;
    border-bottom: 1px solid var(--border-faint);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  th button {
    background: none;
    border: none;
    color: inherit;
    font: inherit;
    cursor: pointer;
    padding: 0;
  }
  /* The sort arrow takes room only on the sorted column. */
  /* A sortable header keeps room on its right for the sort arrow, which sits
     in that padding: sorting never widens or clips the label. */
  thead th[aria-sort] {
    padding-right: 10px;
  }
  .arrow {
    position: absolute;
    right: 1px;
    top: 50%;
    transform: translateY(-50%);
    font-size: var(--fs-xs);
  }
  th.sel input,
  td.sel input {
    margin: 0;
    vertical-align: middle;
  }
  tbody tr {
    cursor: default;
  }
  /* Every data row is one line of the same height (spec §3.1.3): cells never
     wrap, and their tallest content (a 20px chip) fits the declared height. */
  tbody tr.row {
    height: var(--history-row-h);
  }
  tbody tr.row td {
    padding-top: 0;
    padding-bottom: 0;
  }
  tr.pad td {
    padding: 0;
    border: 0;
  }
  td.pad-top {
    height: var(--vt-before, 0px);
  }
  td.pad-bottom {
    height: var(--vt-after, 0px);
  }
  tbody tr:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }
  .list:focus-visible {
    outline: none;
  }
  .history :global(.search) {
    width: 16rem;
    max-width: 100%;
  }
  tbody tr:hover td {
    background: var(--bg-hover);
  }
  td.open {
    cursor: pointer;
  }
  td.host,
  td.path {
    font-family: var(--font-mono);
  }
  /* Slimmer badges in the table rows. */
  td :global(.badge) {
    padding: 0 4px;
  }
  /* A zero max-width lets Host and Path shrink below their text and ellipsize. */
  td.host,
  td.path {
    max-width: 0;
  }
  td.time,
  td.num {
    font-variant-numeric: tabular-nums;
    color: var(--fg-muted);
  }
  .num {
    text-align: right;
  }
  .idx {
    color: var(--fg-faint);
  }
  tr.checked td {
    background: var(--accent-soft);
  }
  tr.selected td {
    background: var(--row-selected);
  }
  tr.selected td:first-child {
    box-shadow: inset 2px 0 0 var(--accent);
  }
  tr.uncommitted td {
    font-style: italic;
  }
  .dot {
    color: var(--warn);
    margin-right: var(--s-1);
  }
  .chips {
    display: flex;
    flex-wrap: nowrap;
    gap: var(--s-1);
    min-width: 0;
    max-width: 6rem;
    overflow: hidden;
  }
  /* Chips shrink and ellipsize (Chip's text already clips with an ellipsis). */
  .chips :global(.chip) {
    flex: 0 1 auto;
    min-width: 0;
  }
  .chips :global(.chip .text) {
    min-width: 0;
  }
  .more {
    display: none;
    flex: none;
    font-size: var(--fs-xs);
    color: var(--fg-muted);
  }
  .empty {
    color: var(--fg-faint);
    padding: var(--s-4);
    text-align: center;
  }
  .foot {
    position: sticky;
    bottom: 0;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--s-2);
    gap: var(--s-2);
    background: var(--bg-elev);
    border-top: 1px solid var(--border);
  }
  .count {
    font-size: var(--fs-sm);
    color: var(--fg-muted);
    white-space: nowrap;
  }
  .cap-note {
    flex: 1 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--fs-sm);
    color: var(--warn-fg);
  }
  .error {
    color: var(--danger);
    margin: 0;
    font-size: var(--fs-md);
  }
  .detail-pane {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-width: 0;
    min-height: 0;
    border-left: 1px solid var(--border);
  }
  .detail-body {
    flex: 1 1 auto;
    min-height: 0;
    overflow: auto;
  }
</style>
