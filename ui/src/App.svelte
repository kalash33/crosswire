<script lang="ts">
  import { onMount } from 'svelte';
  import { session } from './lib/stores/session.svelte';
  import Auth from './routes/Auth.svelte';

  const appName = 'Crosswire';

  onMount(() => {
    // A bootstrap fragment wins; otherwise check for an existing cookie session.
    if (location.hash.includes('bootstrap=')) void session.bootstrapFromFragment();
    else void session.probe();
  });
</script>

<div data-testid="app-root" class="app-root">
  {#if session.status === 'unknown'}
    <p data-testid="loading">Loading…</p>
  {:else if session.status === 'authenticated'}
    <div data-testid="app-shell">
      <h1>{appName}</h1>
      <p>Web UI foundation.</p>
    </div>
  {:else}
    <Auth />
  {/if}
</div>
