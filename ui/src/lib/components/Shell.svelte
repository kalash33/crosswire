<script lang="ts">
  import { onMount, type Snippet } from 'svelte';
  import { keymap } from '../keymap';
  import { router, routes } from '../router.svelte';
  import { intercept } from '../stores/intercept.svelte';
  import { selectionBytes } from '../decode/selection';
  import DecoderPane from './DecoderPane.svelte';
  import StatusBar from './StatusBar.svelte';

  let { children }: { children?: Snippet } = $props();

  // decode-selection opens the decoder pane seeded from the live DOM selection
  // (covers CodeMirror and plain text); an empty selection opens it empty.
  let decoderInput = $state<Uint8Array | null>(null);

  // The ONE keydown dispatch for the whole app.
  onMount(() => {
    const on = (e: KeyboardEvent) => keymap.dispatch(e);
    window.addEventListener('keydown', on);

    // Global chords (available from every view). toggle-intercept flips the
    // proxy hold from anywhere; decode-selection opens the decoder pane for the
    // current selection. View-scoped chords (send, forward, drop, tab cycling,
    // send-to-repeater) are registered by the view that owns them and torn down
    // on unmount, so a chord never acts on a view that is no longer mounted.
    const offToggle = keymap.register('toggle-intercept', () => {
      if (intercept.loaded) void intercept.setEnabled(!intercept.enabled);
    });
    const offDecode = keymap.register('decode-selection', () => {
      decoderInput = selectionBytes();
    });

    return () => {
      window.removeEventListener('keydown', on);
      offToggle();
      offDecode();
    };
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

  {#if decoderInput}
    {#key decoderInput}
      <DecoderPane initial={decoderInput} onClose={() => (decoderInput = null)} />
    {/key}
  {/if}

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
