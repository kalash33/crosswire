<script lang="ts">
  import { onMount } from 'svelte';
  import ScopeRules from '../lib/components/ScopeRules.svelte';
  import ScopeImport from '../lib/components/ScopeImport.svelte';
  import { scope } from '../lib/stores/scope.svelte';

  onMount(() => {
    scope.clearError(); // a stale error from History's add-host must not show here
    void scope.load();
  });
</script>

<div class="scope" data-testid="scope-view">
  <header class="top">
    <label class="switch">
      <input
        type="checkbox"
        role="switch"
        data-testid="strict-toggle"
        checked={scope.enforceStrict}
        onchange={(e) => void scope.setEnforceStrict(e.currentTarget.checked)}
      />
      <span class="track" aria-hidden="true"><span class="thumb"></span></span>
      <span class="name">Enforce scope strictly</span>
    </label>
    {#if !scope.strictKnown}
      <span class="hint">Current server state unknown; the API has no read.</span>
    {/if}
  </header>

  <ScopeRules />
  <ScopeImport />
</div>

<style>
  .scope {
    display: flex;
    flex-direction: column;
    gap: var(--s-3);
    max-width: 60rem;
    padding: var(--s-4);
  }
  .top {
    display: flex;
    align-items: center;
    gap: var(--s-3);
    padding: var(--s-3) var(--s-4);
    background: var(--bg-elev);
    border: 1px solid var(--border);
    border-radius: var(--r-2);
  }
  .switch {
    display: inline-flex;
    align-items: center;
    gap: var(--s-3);
    cursor: pointer;
  }
  .switch input {
    position: absolute;
    opacity: 0;
    width: 1px;
    height: 1px;
  }
  .track {
    position: relative;
    width: 40px;
    height: 22px;
    border-radius: 11px;
    background: var(--bg-active);
    border: 1px solid var(--border-strong);
  }
  .thumb {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: var(--fg-muted);
    transition: transform 0.12s;
  }
  .switch input:checked + .track {
    background: var(--ok-bg);
    border-color: var(--ok);
  }
  .switch input:checked + .track .thumb {
    transform: translateX(18px);
    background: var(--ok);
  }
  .switch input:focus-visible + .track {
    outline: 2px solid var(--focus);
    outline-offset: 2px;
  }
  .name {
    font-weight: 600;
  }
  .hint {
    color: var(--fg-faint);
    font-size: var(--fs-sm);
  }
</style>
