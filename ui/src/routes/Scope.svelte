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
  <label class="strict">
    <input
      type="checkbox"
      role="switch"
      data-testid="strict-toggle"
      checked={scope.enforceStrict}
      onchange={(e) => void scope.setEnforceStrict(e.currentTarget.checked)}
    />
    Enforce scope strictly
    {#if !scope.strictKnown}<span class="hint">(current server state unknown; the API has no read)</span>{/if}
  </label>

  <h2>Rules</h2>
  <ScopeRules />

  <h2>Import program list</h2>
  <ScopeImport />
</div>

<style>
  .scope {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    max-width: 60rem;
  }
  h2 {
    font-size: 1rem;
    margin: 0.5rem 0 0;
  }
  .hint {
    opacity: 0.6;
    font-size: 0.75rem;
  }
</style>
