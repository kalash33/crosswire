<script lang="ts">
  import { onMount } from 'svelte';
  import { apiGet } from '../api/client';
  import type { CountsDto } from '../api/types';
  import { events } from '../stores/events.svelte';

  import { intercept } from '../stores/intercept.svelte';

  import { scope } from '../stores/scope.svelte';

  let counts = $state<CountsDto | null>(null);
  let failed = $state(false);

  async function refreshCounts() {
    try {
      counts = await apiGet<CountsDto>('/api/v1/counts');
    } catch {
      failed = true;
    }
  }
  async function refreshScope() {
    await scope.load();
    if (scope.error) failed = true;
  }
  function refreshAll() {
    failed = false;
    void refreshCounts();
    void intercept.refresh();
    void refreshScope();
  }

  onMount(() => {
    refreshAll();
    // The shared intercept store keeps itself in sync (events + lagged).
    intercept.start();
    const off = events.subscribe((e) => {
      if (e.type === 'committed') void refreshCounts();
    }, refreshAll);
    return () => {
      off();
      intercept.stop();
    };
  });

  const scopeLabel = $derived(
    !scope.loaded
      ? '…'
      : scope.scope.include.length + scope.scope.exclude.length === 0
        ? 'none'
        : `${scope.scope.include.length} in / ${scope.scope.exclude.length} out`,
  );
</script>

<!-- One pinned line in the shell's status row: never wraps, clips instead. -->
<footer class="statusbar" data-testid="status-bar">
  <span class="item" data-testid="status-api">API {location.host}</span>
  <!-- TODO: proxy bind address needs a future endpoint (no GET /settings in alpha.2). -->
  <span class="item" data-testid="status-proxy">Proxy n/a</span>
  <span
    class="item intercept"
    class:on={intercept.loaded && intercept.enabled}
    data-testid="status-intercept"
    data-enabled={intercept.loaded ? intercept.enabled : ''}
  >
    <span class="dot" aria-hidden="true"></span>Intercept {intercept.loaded ? (intercept.enabled ? 'ON' : 'OFF') : '…'}{#if intercept.held.length > 0}{` (${intercept.held.length} held)`}{/if}
  </span>
  <button
    type="button"
    class="toggle"
    data-testid="intercept-toggle"
    role="switch"
    aria-checked={intercept.enabled}
    disabled={!intercept.loaded}
    onclick={() => void intercept.setEnabled(!intercept.enabled)}
  >
    {intercept.enabled ? 'Turn intercept off' : 'Turn intercept on'}
  </button>
  <span class="item" data-testid="status-scope">Scope {scopeLabel}{#if scope.strictKnown} · strict {scope.enforceStrict ? 'ON' : 'OFF'}{/if}</span>
  <span class="item counts" data-testid="status-counts">
    {#if counts}History {counts.history} · Repeater {counts.repeater} · Intruder {counts.intruder}{:else}Counts …{/if}
  </span>
  {#if failed}<span class="item error" data-testid="status-error">status unavailable</span>{/if}
</footer>

<style>
  .statusbar {
    grid-area: status;
    display: flex;
    align-items: center;
    gap: var(--s-4);
    height: var(--statusbar-h);
    min-width: 0;
    padding: 0 var(--s-3);
    font-size: var(--fs-sm);
    line-height: 1;
    color: var(--fg-muted);
    white-space: nowrap;
    overflow: hidden;
    background: var(--bg-elev);
    border-top: 1px solid var(--border);
  }
  .item {
    flex: none;
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .counts {
    flex: 0 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--fg-faint);
  }
  .intercept.on {
    color: var(--warn);
  }
  .intercept.on .dot {
    background: var(--warn);
  }
  .toggle {
    flex: none;
    height: 20px;
    min-height: 0;
    padding: 0 var(--s-2);
    font-size: var(--fs-xs);
    color: var(--fg);
    background: var(--bg);
    border: 1px solid var(--border-strong);
    border-radius: var(--r-1);
    cursor: pointer;
  }
  .toggle:hover:not(:disabled) {
    background: var(--bg-hover);
  }
  .error {
    color: var(--danger);
  }
</style>
