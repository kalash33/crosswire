<script lang="ts">
  import { onMount } from 'svelte';
  import { apiGet } from '../api/client';
  import type { CountsDto } from '../api/types';
  import { events } from '../stores/events.svelte';

  import { intercept } from '../stores/intercept.svelte';

  // Minimal local shape: only what the bar reads. GET /scope is untyped in
  // openapi.json; its full type belongs to the scope task.
  interface ScopeDoc {
    include: unknown[];
    exclude: unknown[];
  }

  let counts = $state<CountsDto | null>(null);
  let scope = $state<ScopeDoc | null>(null);
  let failed = $state(false);

  async function refreshCounts() {
    try {
      counts = await apiGet<CountsDto>('/api/v1/counts');
    } catch {
      failed = true;
    }
  }
  async function refreshScope() {
    try {
      scope = await apiGet<ScopeDoc>('/api/v1/scope');
    } catch {
      failed = true;
    }
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
    scope === null
      ? '…'
      : scope.include.length + scope.exclude.length === 0
        ? 'none'
        : `${scope.include.length} in / ${scope.exclude.length} out`,
  );
</script>

<footer class="statusbar" data-testid="status-bar">
  <span data-testid="status-api">API {location.host}</span>
  <!-- TODO: proxy bind address needs a future endpoint (no GET /settings in alpha.2). -->
  <span data-testid="status-proxy">Proxy n/a</span>
  <span data-testid="status-intercept" data-enabled={intercept.loaded ? intercept.enabled : ''}>
    Intercept {intercept.loaded ? (intercept.enabled ? 'ON' : 'OFF') : '…'}{#if intercept.held.length > 0}
      ({intercept.held.length} held){/if}
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
  <span data-testid="status-scope">Scope {scopeLabel}</span>
  <span data-testid="status-counts">
    {#if counts}History {counts.history} · Repeater {counts.repeater} · Intruder {counts.intruder}{:else}Counts …{/if}
  </span>
  {#if failed}<span data-testid="status-error">status unavailable</span>{/if}
</footer>

<style>
  .statusbar {
    display: flex;
    gap: 1.25rem;
    flex-wrap: wrap;
    padding: 0.25rem 0.75rem;
    font-size: 0.8rem;
    border-top: 1px solid #8884;
    align-items: center;
  }
  .toggle {
    font: inherit;
    font-size: 0.75rem;
    cursor: pointer;
  }
</style>
