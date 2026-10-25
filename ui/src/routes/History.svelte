<script lang="ts">
  import { onMount } from 'svelte';
  import { SvelteSet } from 'svelte/reactivity';
  import { history, type ExchangeFilter, type Row } from '../lib/stores/history.svelte';
  import { events } from '../lib/stores/events.svelte';
  import ExchangeDetail from '../lib/components/ExchangeDetail.svelte';

  // ---- Filter inputs (applied on submit; the store builds the query) --------
  let fHost = $state('');
  let fHostContains = $state('');
  let fMethod = $state('');
  let fStatusMin = $state('');
  let fStatusMax = $state('');
  let fTag = $state('');
  let fState = $state('');
  let fScopeState = $state('');

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

  // ---- Detail selection ------------------------------------------------------
  let selectedId = $state<string | null>(null);

  function openRow(id: string): void {
    selectedId = id;
  }

  function fmtTime(ms: number): string {
    try {
      return new Date(ms).toLocaleString();
    } catch {
      return String(ms);
    }
  }

  function arrow(key: SortKey): string {
    if (sortKey !== key) return '';
    return sortDir === 1 ? ' ▲' : ' ▼';
  }

  onMount(() => {
    events.start();
    void history.applyFilter({});
  });
</script>

<div class="history" data-testid="history-view">
  <form class="filters" data-testid="history-filters" onsubmit={applyFilters}>
    <input data-testid="filter-host" placeholder="host (exact)" bind:value={fHost} />
    <input
      data-testid="filter-host-contains"
      placeholder="host contains"
      bind:value={fHostContains}
    />
    <input data-testid="filter-method" placeholder="method" bind:value={fMethod} />
    <input
      data-testid="filter-status-min"
      type="number"
      placeholder="status ≥"
      bind:value={fStatusMin}
    />
    <input
      data-testid="filter-status-max"
      type="number"
      placeholder="status ≤"
      bind:value={fStatusMax}
    />
    <input data-testid="filter-tag" placeholder="tag" bind:value={fTag} />
    <select data-testid="filter-state" bind:value={fState} aria-label="recording state">
      <option value="">state: any</option>
      <option value="received">received</option>
      <option value="committed">committed</option>
      <option value="error">error</option>
    </select>
    <select data-testid="filter-scope-state" bind:value={fScopeState} aria-label="scope state">
      <option value="">scope: any</option>
      <option value="in">in</option>
      <option value="out">out</option>
      <option value="empty">empty</option>
      <option value="host-mismatch">host-mismatch</option>
    </select>
    <button type="submit" data-testid="filter-apply">Apply</button>
    <button type="button" data-testid="filter-clear" onclick={clearFilters}>Clear</button>
  </form>

  {#if history.error}
    <p class="error" data-testid="history-error" role="alert">{history.error}</p>
  {/if}

  <div class="layout">
    <div class="list" data-testid="history-list">
      <table>
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
            <th>#</th>
            <th><button type="button" onclick={() => sortBy('created_at')}>Time{arrow('created_at')}</button></th>
            <th><button type="button" onclick={() => sortBy('method')}>Method{arrow('method')}</button></th>
            <th><button type="button" onclick={() => sortBy('host')}>Host{arrow('host')}</button></th>
            <th><button type="button" onclick={() => sortBy('path')}>Path{arrow('path')}</button></th>
            <th><button type="button" onclick={() => sortBy('status')}>Status{arrow('status')}</button></th>
            <th><button type="button" onclick={() => sortBy('resp_len')}>Length{arrow('resp_len')}</button></th>
            <th><button type="button" onclick={() => sortBy('state')}>State{arrow('state')}</button></th>
            <th>Tags</th>
          </tr>
        </thead>
        <tbody>
          {#each sortedRows as row, i (row.id)}
            <tr
              class:uncommitted={row.uncommitted}
              class:selected={selectedId === row.id}
              data-testid="history-row"
              data-id={row.id}
              data-uncommitted={row.uncommitted}
            >
              <td class="sel">
                <input
                  type="checkbox"
                  aria-label={`select ${row.id}`}
                  checked={selected.has(row.id)}
                  onchange={() => toggleRow(row.id)}
                />
              </td>
              <td class="num">{i + 1}</td>
              <td class="open" onclick={() => openRow(row.id)}>
                {#if row.uncommitted}<span
                    class="dot"
                    title="Uncommitted (not yet durable)"
                    data-testid="uncommitted-marker">●</span
                  >{/if}
                {fmtTime(row.created_at)}
              </td>
              <td class="open" onclick={() => openRow(row.id)}>{row.method ?? ''}</td>
              <td class="open host" onclick={() => openRow(row.id)}>{row.host ?? ''}</td>
              <td class="open path" onclick={() => openRow(row.id)}>{row.path ?? ''}</td>
              <td class="open num" onclick={() => openRow(row.id)}>{row.status ?? ''}</td>
              <td class="open num" onclick={() => openRow(row.id)}
                >{row.resp_len ?? ''}</td
              >
              <td class="open" onclick={() => openRow(row.id)}>{row.state}</td>
              <td class="tags">
                {#each row.tags as tag (tag)}<span class="tag">{tag}</span>{/each}
              </td>
            </tr>
          {/each}
          {#if sortedRows.length === 0 && !history.loading}
            <tr><td colspan="10" class="empty" data-testid="history-empty">No exchanges.</td></tr>
          {/if}
        </tbody>
      </table>

      <div class="foot">
        <span class="count">{sortedRows.length} loaded · {selected.size} selected</span>
        <button
          type="button"
          data-testid="load-more"
          disabled={!history.hasMore || history.loading}
          onclick={() => void history.loadMore()}
        >
          {history.loading ? 'Loading…' : history.hasMore ? 'Load more' : 'End'}
        </button>
      </div>

      <!-- Beta-only row actions: disabled stubs, never faked (spec §7.0). -->
      <div class="actions" data-testid="row-actions" aria-label="row actions (beta)">
        <button type="button" disabled title="Available in a later release"
          >Send to Repeater</button
        >
        <button type="button" disabled title="Available in a later release">Copy as curl</button>
        <button type="button" disabled title="Available in a later release">Tag…</button>
      </div>
    </div>

    <aside class="detail-pane" data-testid="history-detail">
      {#if selectedId}
        {#key selectedId}
          <ExchangeDetail id={selectedId} />
        {/key}
      {:else}
        <p class="placeholder">Select a row to view the exchange.</p>
      {/if}
    </aside>
  </div>
</div>

<style>
  .history {
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
    min-width: 0;
  }
  .filters {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
    align-items: center;
  }
  .filters input,
  .filters select {
    padding: 0.25rem 0.4rem;
    font-size: 0.8rem;
  }
  .filters input[type='number'] {
    width: 5.5rem;
  }
  .layout {
    display: grid;
    grid-template-columns: minmax(0, 3fr) minmax(0, 2fr);
    gap: 0.75rem;
    align-items: start;
  }
  @media (max-width: 860px) {
    .layout {
      grid-template-columns: 1fr;
    }
  }
  .list {
    min-width: 0;
    overflow: auto;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.8rem;
  }
  th,
  td {
    text-align: left;
    padding: 0.2rem 0.4rem;
    border-bottom: 1px solid #8883;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 18rem;
  }
  th button {
    background: none;
    border: none;
    color: inherit;
    font: inherit;
    font-weight: 600;
    cursor: pointer;
    padding: 0;
  }
  td.open {
    cursor: pointer;
  }
  td.num {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  .sel {
    width: 1.5rem;
  }
  tr.selected {
    background: #26263a;
  }
  tr.uncommitted td {
    font-style: italic;
    opacity: 0.85;
  }
  .dot {
    color: #ffb454;
    margin-right: 0.25rem;
  }
  .tag {
    display: inline-block;
    margin-right: 0.2rem;
    padding: 0 0.3rem;
    border-radius: 3px;
    background: #223322;
    color: #bfe9bf;
    font-size: 0.72rem;
  }
  .empty,
  .placeholder {
    opacity: 0.6;
    padding: 0.75rem;
  }
  .foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0.4rem 0;
    gap: 0.5rem;
  }
  .count {
    font-size: 0.78rem;
    opacity: 0.7;
  }
  .actions {
    display: flex;
    gap: 0.4rem;
  }
  .actions button {
    font-size: 0.75rem;
  }
  .error {
    color: #ff9b9b;
    margin: 0;
    font-size: 0.85rem;
  }
  .detail-pane {
    min-width: 0;
    border-left: 1px solid #8883;
    padding-left: 0.75rem;
  }
  @media (max-width: 860px) {
    .detail-pane {
      border-left: none;
      padding-left: 0;
      border-top: 1px solid #8883;
      padding-top: 0.75rem;
    }
  }
</style>
