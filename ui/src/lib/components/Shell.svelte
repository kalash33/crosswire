<script lang="ts">
  import { onMount, type Snippet } from 'svelte';
  import { keymap } from '../keymap';
  import { router } from '../router.svelte';
  import { intercept } from '../stores/intercept.svelte';
  import { events } from '../stores/events.svelte';
  import { decoder } from '../stores/decoder.svelte';
  import { selectionBytes } from '../decode/selection';
  import Rail from './Rail.svelte';
  import TopBar from './TopBar.svelte';
  import StatusBar from './StatusBar.svelte';
  import ShortcutsDialog from './ShortcutsDialog.svelte';
  import Toast from '../ui/Toast.svelte';
  import { toasts } from '../ui/toasts.svelte';

  let { children }: { children?: Snippet } = $props();

  let shortcutsOpen = $state(false);

  // Live-socket transitions, one toast each (never per retry): "lost" once
  // after a connected socket drops, "reconnected" once when it comes back.
  // The very first connect is silent.
  let everConnected = false;
  let lost = false;
  $effect(() => {
    if (events.connected) {
      if (lost) toasts.push({ kind: 'success', text: 'Live updates reconnected.' });
      lost = false;
      everConnected = true;
    } else if (everConnected && !lost) {
      lost = true;
      toasts.push({ kind: 'error', text: 'Live updates lost. Reconnecting…', ttlMs: 6000 });
    }
  });

  // The ONE keydown dispatch for the whole app.
  onMount(() => {
    // While the cheat sheet is open the page behind it is inert: no global
    // chord (Ctrl+T, Ctrl+Shift+D, …) may act on it.
    const on = (e: KeyboardEvent) => {
      if (!shortcutsOpen) keymap.dispatch(e);
    };
    window.addEventListener('keydown', on);
    // Every route gets the live socket (status dot, counts, intercept queue).
    events.start();

    // Global chords (available from every view). toggle-intercept flips the
    // proxy hold from anywhere; decode-selection seeds the Decoder route with
    // the current selection and opens it; `?` opens the cheat sheet. View-
    // scoped chords (send, forward, drop, tab cycling, send-to-repeater) are
    // registered by the view that owns them and torn down on unmount, so a
    // chord never acts on a view that is no longer mounted.
    const offToggle = keymap.register('toggle-intercept', () => {
      if (intercept.loaded) void intercept.setEnabled(!intercept.enabled);
    });
    const offDecode = keymap.register('decode-selection', () => {
      const sel = selectionBytes();
      decoder.seed(sel.bytes, sel.nonLatin1);
      router.navigate('/decoder');
    });
    const offShortcuts = keymap.register('show-shortcuts', () => {
      shortcutsOpen = true;
    });

    return () => {
      window.removeEventListener('keydown', on);
      offToggle();
      offDecode();
      offShortcuts();
      events.close();
    };
  });
</script>

<div class="shell" data-testid="app-shell">
  <Rail />
  <TopBar onShortcuts={() => (shortcutsOpen = true)} />
  <main class="main" data-testid="app-main">
    {@render children?.()}
  </main>
  <StatusBar />
  <Toast />
  {#if shortcutsOpen}
    <ShortcutsDialog onClose={() => (shortcutsOpen = false)} />
  {/if}
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
  .main {
    grid-area: main;
    min-width: 0;
    min-height: 0;
    overflow: auto;
    padding: var(--s-3) var(--s-4);
  }
</style>
