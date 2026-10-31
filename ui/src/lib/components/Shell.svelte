<script lang="ts">
  import { onMount, type Snippet } from 'svelte';
  import { keymap } from '../keymap';
  import { router, routes } from '../router.svelte';
  import { intercept } from '../stores/intercept.svelte';
  import { selectionBytes } from '../decode/selection';
  import DecoderPane from './DecoderPane.svelte';
  import StatusBar from './StatusBar.svelte';
  import Icon from '../ui/Icon.svelte';
  import Toast from '../ui/Toast.svelte';
  import type { IconName } from '../ui/icons';

  let { children }: { children?: Snippet } = $props();

  const ICON_FOR: Record<string, IconName> = {
    '/': 'history',
    '/repeater': 'repeater',
    '/intercept': 'intercept',
    '/scope': 'scope',
  };
  const currentLabel = $derived(routes.find((r) => r.path === router.path)?.label ?? '');

  // decode-selection opens the decoder pane seeded from the live DOM selection
  // (covers CodeMirror and plain text); an empty selection opens it empty.
  let decoderInput = $state<{ bytes: Uint8Array; nonLatin1: boolean } | null>(null);

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
  <nav class="rail" data-testid="app-nav" aria-label="Primary">
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
        <Icon name={ICON_FOR[route.path] ?? 'chevron-right'} size={18} />
        <span class="nav-label">{route.label}</span>
      </button>
    {/each}
  </nav>
  <!-- Placeholder top bar: Task 1 replaces it with TopBar.svelte. -->
  <header class="top">
    <strong class="brand">Crosswire</strong>
    <span class="here">{currentLabel}</span>
  </header>
  <main class="main" data-testid="app-main">
    {@render children?.()}
  </main>

  {#if decoderInput}
    {#key decoderInput}
      <DecoderPane initial={decoderInput.bytes} initialNonLatin1={decoderInput.nonLatin1} onClose={() => (decoderInput = null)} />
    {/key}
  {/if}

  <StatusBar />
  <Toast />
</div>

<style>
  /* Fixed-height application grid (U1 spec D3): only .main scrolls. */
  .shell {
    display: grid;
    grid-template-columns: var(--rail-w) minmax(0, 1fr);
    grid-template-rows: var(--topbar-h) minmax(0, 1fr) var(--statusbar-h);
    grid-template-areas:
      'rail top'
      'rail main'
      'status status';
    height: 100vh;
    height: 100dvh;
    overflow: hidden;
    background: var(--bg);
  }
  .rail {
    grid-area: rail;
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 2px;
    padding: var(--s-1);
    min-height: 0;
    overflow-y: auto;
    background: var(--bg-elev);
    border-right: 1px solid var(--border);
  }
  .nav-link {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 2px;
    height: 46px;
    min-height: 0;
    padding: 0;
    color: var(--fg-muted);
    background: transparent;
    border: none;
    border-radius: var(--r-1);
    cursor: pointer;
    position: relative;
  }
  .nav-link:hover:not(.active) {
    color: var(--fg);
    background: var(--bg-hover);
  }
  .nav-link.active {
    color: var(--fg);
    background: var(--accent-soft);
  }
  .nav-link.active::before {
    content: '';
    position: absolute;
    left: calc(-1 * var(--s-1));
    top: 10px;
    bottom: 10px;
    width: 2px;
    border-radius: 0 2px 2px 0;
    background: var(--accent);
  }
  .nav-link.active :global(.icon) {
    color: var(--accent);
  }
  .nav-label {
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    font-size: 10px;
    line-height: 1.1;
    letter-spacing: 0.01em;
  }
  .top {
    grid-area: top;
    display: flex;
    align-items: center;
    gap: var(--s-3);
    min-width: 0;
    padding: 0 var(--s-4);
    background: var(--bg-elev);
    border-bottom: 1px solid var(--border);
  }
  .brand {
    font-size: var(--fs-lg);
    font-weight: 650;
    letter-spacing: -0.01em;
  }
  .here {
    color: var(--fg-muted);
    padding-left: var(--s-3);
    border-left: 1px solid var(--border);
  }
  .main {
    grid-area: main;
    min-width: 0;
    min-height: 0;
    overflow: auto;
    padding: var(--s-3) var(--s-4);
  }
</style>
