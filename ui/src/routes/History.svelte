<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { SvelteSet } from 'svelte/reactivity';
  import { history, type ExchangeFilter, type Row } from '../lib/stores/history.svelte';
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
  import { humanBytes, timeShort } from '../lib/format';
  import Badge from '../lib/ui/Badge.svelte';
  import Button from '../lib/ui/Button.svelte';
  import Chip from '../lib/ui/Chip.svelte';
  import EmptyState from '../lib/ui/EmptyState.svelte';
  import Input from '../lib/ui/Input.svelte';
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

  const sortedRows = $derived([...history.rows].sort(cmp));

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

  function toggleAll(): void {
    if (selected.size === sortedRows.length) selected.clear();
    else {
      selected.clear();
      for (const r of sortedRows) selected.add(r.id);
    }
  }

  // Checked ids in table order (the order bulk PATCHes are sent in).
  const checkedIds = $derived(sortedRows.filter((r) => selected.has(r.id)).map((r) => r.id));

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

  // Enter/Space opens the focused row (keyboard-operable rows, §7.6/§11.1): the
  // mouse-free loop selects an exchange without a click. Space is prevented so
  // the page does not scroll.
  function onRowKey(e: KeyboardEvent, id: string): void {
    if (e.key === 'Enter' || e.key === ' ' || e.key === 'Spacebar') {
      e.preventDefault();
      openRow(id);
    }
  }

  const rowById = (id: string | null): Row | null => (id ? (history.rows.find((r) => r.id === id) ?? null) : null);
  const selectedRow = $derived(rowById(selectedId));
  // One action target for the toolbar AND the Ctrl+R chord: the focused row,
  // then the opened row. The toolbar names it when it is not the open row.
  const target = $derived(actionTarget(focusedId, selectedId, sortedRows));
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

  function onRowContextMenu(e: MouseEvent, id: string): void {
    e.preventDefault();
    menuRowEl = e.currentTarget as HTMLElement;
    // The context-menu key reports (0,0): place the menu at the row instead.
    if (e.clientX === 0 && e.clientY === 0) {
      const r = menuRowEl.getBoundingClientRect();
      menuAnchor = { x: r.left + 24, y: r.bottom };
    } else {
      menuAnchor = { x: e.clientX, y: e.clientY };
    }
    menuId = id;
  }

  function closeMenu(): void {
    menuId = null;
    menuRowEl?.focus();
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

<div class="history" data-testid="history-view">
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
    <Splitter direction="row" storageKey="history-detail" initial={0.6} min={[320, 360]} label="Resize history and detail">
      {#snippet a()}
        <div class="list" data-testid="history-list">
          <table>
            <colgroup>
              <col class="c-sel" />
              <col class="c-idx" />
              <col class="c-time" />
              <col class="c-method" />
              <col class="c-host" />
              <col />
              <col class="c-status" />
              <col class="c-len" />
              <col class="c-state" />
              <col class="c-tags" />
            </colgroup>
            <thead>
              <tr>
                <th class="sel">
                  <input
                    type="checkbox"
                    aria-label="select all"
                    data-testid="select-all"
                    checked={selected.size > 0 && selected.size === sortedRows.length}
                    onchange={toggleAll}
                  />
                </th>
                <th class="num">#</th>
                {@render sortHead('created_at', 'Time')}
                {@render sortHead('method', 'Method')}
                {@render sortHead('host', 'Host')}
                {@render sortHead('path', 'Path')}
                {@render sortHead('status', 'Status')}
                {@render sortHead('resp_len', 'Length', 'num')}
                {@render sortHead('state', 'State')}
                <th>Tags</th>
              </tr>
            </thead>
            <tbody>
              {#each sortedRows as row, i (row.id)}
                <!-- Keyboard-operable row (§7.6): focusable, Enter/Space opens it,
                     focusin marks it as the action target, the context-menu key
                     or a right-click opens the row menu. -->
                <tr
                  class:uncommitted={row.uncommitted}
                  class:selected={selectedId === row.id}
                  class:checked={selected.has(row.id)}
                  data-testid="history-row"
                  data-id={row.id}
                  data-uncommitted={row.uncommitted}
                  tabindex="0"
                  aria-label={`exchange ${row.method ?? ''} ${row.host ?? ''}${row.path ?? ''}`}
                  onkeydown={(e) => onRowKey(e, row.id)}
                  onfocusin={() => (focusedId = row.id)}
                  oncontextmenu={(e) => onRowContextMenu(e, row.id)}
                >
                  <td class="sel">
                    <input
                      type="checkbox"
                      aria-label={`select ${row.id}`}
                      checked={selected.has(row.id)}
                      onchange={() => toggleRow(row.id)}
                    />
                  </td>
                  <td class="num idx">{i + 1}</td>
                  <td class="open time" title={fullTime(row.created_at)} onclick={() => openRow(row.id)}>
                    {#if row.uncommitted}<span class="dot" title="Uncommitted (not yet durable)" data-testid="uncommitted-marker"
                        >●</span
                      >{/if}{timeShort(row.created_at)}
                  </td>
                  <td class="open" onclick={() => openRow(row.id)}>
                    {#if row.method}<Badge kind="method" value={row.method} />{/if}
                  </td>
                  <td class="open host" title={row.host ?? ''} onclick={() => openRow(row.id)}>{row.host ?? ''}</td>
                  <td class="open path" title={row.path ?? ''} onclick={() => openRow(row.id)}>{row.path ?? ''}</td>
                  <td class="open" onclick={() => openRow(row.id)}>
                    {#if row.status !== null}<Badge kind="status" value={row.status} />{/if}
                  </td>
                  <td
                    class="open num"
                    title={row.resp_len === null ? '' : `${row.resp_len} bytes`}
                    onclick={() => openRow(row.id)}>{row.resp_len === null ? '' : humanBytes(row.resp_len)}</td
                  >
                  <td class="open" onclick={() => openRow(row.id)}><Badge kind="state" value={row.state} /></td>
                  <td class="tags" title={row.tags.join(', ')}>
                    <span class="chips">{#each row.tags as tag (tag)}<Chip>{tag}</Chip>{/each}</span>
                  </td>
                </tr>
              {/each}
              {#if sortedRows.length === 0 && !history.loading}
                <tr>
                  <td colspan="10" class="empty" data-testid="history-empty">
                    {activeFilters > 0 ? 'No exchanges match these filters.' : 'No exchanges.'}
                  </td>
                </tr>
              {/if}
            </tbody>
          </table>

          <div class="foot">
            <span class="count" data-testid="history-count">{sortedRows.length} loaded · {selected.size} selected</span>
            <Button
              size="sm"
              data-testid="load-more"
              disabled={!history.hasMore || history.loading}
              onclick={() => void history.loadMore()}
            >
              {history.loading ? 'Loading…' : history.hasMore ? 'Load more' : 'End'}
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
          <div class="detail-body" onfocusin={() => (focusedId = selectedId)}>
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
    height: 100%;
    min-width: 0;
    overflow: auto;
  }
  table {
    width: 100%;
    min-width: 820px;
    table-layout: fixed;
    border-collapse: separate;
    border-spacing: 0;
    font-size: var(--fs-sm);
  }
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
    padding: 4px var(--s-2);
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
  .arrow {
    display: inline-block;
    width: 1em;
    font-size: var(--fs-xs);
  }
  tbody tr {
    cursor: default;
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
  .c-sel {
    width: 32px;
  }
  .c-idx {
    width: 40px;
  }
  .c-time {
    width: 72px;
  }
  .c-method {
    width: 76px;
  }
  .c-host {
    width: 18%;
  }
  .c-status {
    width: 56px;
  }
  .c-len {
    width: 68px;
  }
  .c-state {
    width: 88px;
  }
  .c-tags {
    width: 12%;
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
    gap: var(--s-1);
    min-width: 0;
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
