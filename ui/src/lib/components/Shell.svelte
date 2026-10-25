<script lang="ts">
  import { onMount, type Snippet } from 'svelte';
  import { keymap } from '../keymap';
  import { router, routes } from '../router.svelte';
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
  <nav class="nav" data-testid="app-nav" aria-label="Primary">
    <strong class="brand">Crosswire</strong>
    <!-- SPA nav: pushState via router.navigate (no server round-trip), rendered
         as buttons so no dynamic <a href> is introduced (§11.1). -->
    {#each routes as route (route.path)}
      <button
        type="button"
        class="nav-link"
        class:active={router.path === route.path}
        aria-current={router.path === route.path ? 'page' : undefined}
        data-testid={`nav-${route.label.toLowerCase()}`}
        onclick={() => router.navigate(route.path)}
      >
        {route.label}
      </button>
    {/each}
  </nav>
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
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.5rem 0.75rem;
    border-bottom: 1px solid #8884;
  }
  .brand {
    margin-right: 0.5rem;
  }
  .nav-link {
    background: none;
    border: none;
    color: inherit;
    font: inherit;
    font-size: 0.85rem;
    padding: 0.2rem 0.5rem;
    border-radius: 4px;
    cursor: pointer;
    opacity: 0.8;
  }
  .nav-link.active {
    background: #26263a;
    opacity: 1;
  }
  .main {
    overflow: auto;
    padding: 0.75rem;
  }
</style>
