<script lang="ts">
  import { onMount } from 'svelte';
  import { apiGet } from '../api/client';
  import type { CountsDto } from '../api/types';
  import { events } from '../stores/events.svelte';

  // Minimal local shapes: only what the bar reads. GET /intercept and
  // GET /scope are untyped in openapi.json (see InterceptStatus / Scope in
  // the Rust handlers); the full types belong to a later task.
  interface InterceptStatus {
    enabled: boolean;
    held: unknown[];
  }
  interface ScopeDoc {
    include: unknown[];
    exclude: unknown[];
  }

  let counts = $state<CountsDto | null>(null);
  let intercept = $state<InterceptStatus | null>(null);
  let scope = $state<ScopeDoc | null>(null);
  let failed = $state(false);

  async function refreshCounts() {
    try {
      counts = await apiGet<CountsDto>('/api/v1/counts');
    } catch {
      failed = true;
    }
  }
  async function refreshIntercept() {
    try {
      intercept = await apiGet<InterceptStatus>('/api/v1/intercept');
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
    void refreshIntercept();
    void refreshScope();
  }

  onMount(() => {
    refreshAll();
    return events.subscribe((e) => {
      if (e.type === 'committed') void refreshCounts();
      else if (e.type === 'intercept') void refreshIntercept();
    }, refreshAll);
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
  <span data-testid="status-intercept" data-enabled={intercept?.enabled ?? ''}>
    Intercept {intercept === null ? '…' : intercept.enabled ? 'ON' : 'OFF'}{#if intercept && intercept.held.length > 0}
      ({intercept.held.length} held){/if}
  </span>
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
  }
</style>
