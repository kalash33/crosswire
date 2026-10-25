<script lang="ts">
  import { onMount } from 'svelte';
  import { session } from './lib/stores/session.svelte';
  import Auth from './routes/Auth.svelte';
  import Shell from './lib/components/Shell.svelte';
  import History from './routes/History.svelte';
  import Repeater from './routes/Repeater.svelte';
  import { router } from './lib/router.svelte';

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
    <Shell>
      {#if router.path === '/'}
        <History />
      {:else if router.path === '/repeater'}
        <Repeater />
      {:else}
        <p data-testid="route-unknown">No view for {router.path}</p>
      {/if}
    </Shell>
  {:else}
    <Auth />
  {/if}
</div>
