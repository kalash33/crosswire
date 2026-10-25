<script lang="ts">
  import { onMount, type Snippet } from 'svelte';
  import { keymap } from '../keymap';
  import { router, routes } from '../router.svelte';
  import { intercept } from '../stores/intercept.svelte';
  import StatusBar from './StatusBar.svelte';

  let { children }: { children?: Snippet } = $props();

  // ---- decode-selection -----------------------------------------------------
  // A minimal, REAL inline encode of the current selection (Burp's Decoder
  // neighbourhood). The selected text is read from the live DOM selection (which
  // covers the CodeMirror editor and plain text alike) and shown as Base64, URL-
  // and Hex-encoded forms in a small popover. The decoded bytes are rendered as
  // TEXT ONLY (Svelte `{...}` interpolation, never {@html}); §11.1 holds.
  interface Decoded {
    source: string;
    base64: string;
    url: string;
    hex: string;
  }
  let decoded = $state<Decoded | null>(null);

  function encodeSelection(sel: string): Decoded {
    const bytes = new TextEncoder().encode(sel);
    let bin = '';
    let hex = '';
    for (const b of bytes) {
      bin += String.fromCharCode(b);
      hex += b.toString(16).padStart(2, '0');
    }
    return { source: sel, base64: btoa(bin), url: encodeURIComponent(sel), hex };
  }

  // The ONE keydown dispatch for the whole app.
  onMount(() => {
    const on = (e: KeyboardEvent) => keymap.dispatch(e);
    window.addEventListener('keydown', on);

    // Global chords (available from every view). toggle-intercept flips the
    // proxy hold from anywhere; decode-selection pops the encoder for the
    // current selection. View-scoped chords (send, forward, drop, tab cycling,
    // send-to-repeater) are registered by the view that owns them and torn down
    // on unmount, so a chord never acts on a view that is no longer mounted.
    const offToggle = keymap.register('toggle-intercept', () => {
      if (intercept.loaded) void intercept.setEnabled(!intercept.enabled);
    });
    const offDecode = keymap.register('decode-selection', () => {
      const sel = window.getSelection()?.toString() ?? '';
      decoded = sel ? encodeSelection(sel) : null;
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

  {#if decoded}
    <div class="decode-popover" data-testid="decode-popover" role="dialog" aria-label="Decode selection">
      <div class="decode-head">
        <strong>Encode selection</strong>
        <button type="button" aria-label="Dismiss" data-testid="decode-close" onclick={() => (decoded = null)}
          >×</button
        >
      </div>
      <dl class="decode-body">
        <dt>Base64</dt>
        <dd data-testid="decode-base64">{decoded.base64}</dd>
        <dt>URL</dt>
        <dd data-testid="decode-url">{decoded.url}</dd>
        <dt>Hex</dt>
        <dd data-testid="decode-hex">{decoded.hex}</dd>
      </dl>
    </div>
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
  .decode-popover {
    position: fixed;
    right: 0.75rem;
    bottom: 2.5rem;
    z-index: 20;
    max-width: min(32rem, 90vw);
    background: #1c1c28;
    border: 1px solid #3a3a4a;
    border-radius: 6px;
    padding: 0.5rem 0.65rem;
    box-shadow: 0 6px 24px #0008;
    font-size: 0.8rem;
  }
  .decode-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
    margin-bottom: 0.35rem;
  }
  .decode-head button {
    background: none;
    border: none;
    color: inherit;
    cursor: pointer;
    font-size: 1rem;
    line-height: 1;
  }
  .decode-body {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 0.2rem 0.6rem;
    margin: 0;
  }
  .decode-body dt {
    opacity: 0.7;
  }
  .decode-body dd {
    margin: 0;
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    word-break: break-all;
  }
</style>
