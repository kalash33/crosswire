<script lang="ts">
  import { onMount } from 'svelte';
  import { session } from './lib/stores/session.svelte';
  import Auth from './routes/Auth.svelte';
  import Shell from './lib/components/Shell.svelte';
  import { createRouter } from './lib/router.svelte';

  const router = createRouter();

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
      <!-- Task 8 maps "/" to the History view; until then nothing is routed. -->
      <p data-testid="route-path">{router.path}</p>
    </Shell>
  {:else}
    <Auth />
  {/if}
</div>
