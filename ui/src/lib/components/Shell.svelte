<script lang="ts">
  import { onMount, type Snippet } from 'svelte';
  import { keymap } from '../keymap';
  import StatusBar from './StatusBar.svelte';

  let { children }: { children?: Snippet } = $props();

  // The ONE keydown dispatch for the whole app.
  onMount(() => {
    const on = (e: KeyboardEvent) => keymap.dispatch(e);
    window.addEventListener('keydown', on);
    return () => window.removeEventListener('keydown', on);
  });
</script>

<div class="shell" data-testid="app-shell">
  <nav class="nav" data-testid="app-nav" aria-label="Primary"><strong>Crosswire</strong></nav>
  <main class="main" data-testid="app-main">
    {@render children?.()}
  </main>
  <StatusBar />
</div>

<style>
  .shell {
    display: grid;
    grid-template-rows: auto 1fr auto;
    min-height: 100vh;
  }
  .nav {
    padding: 0.5rem 0.75rem;
    border-bottom: 1px solid #8884;
  }
  .main {
    overflow: auto;
    padding: 0.75rem;
  }
</style>
