<script lang="ts">
  import { router, routes } from '../router.svelte';
  import { intercept } from '../stores/intercept.svelte';
  import { events } from '../stores/events.svelte';
  import { connection } from '../stores/connection.svelte';
  import { appearance } from '../stores/appearance.svelte';
  import { keymap, formatChord } from '../keymap';
  import IconButton from '../ui/IconButton.svelte';
  import Icon from '../ui/Icon.svelte';

  let { onShortcuts }: { onShortcuts: () => void } = $props();

  const title = $derived(routes.find((r) => r.path === router.path)?.label ?? '');
  const toggleChord = formatChord(keymap.chordFor('toggle-intercept') ?? '');
  const on = $derived(intercept.loaded && intercept.enabled);
</script>

<header class="top" data-testid="topbar">
  <strong class="brand">Crosswire</strong>
  <span class="here" data-testid="topbar-title">{title}</span>

  <span class="spacer"></span>

  {#if connection.proxyAddr}
    <span class="chip" data-testid="topbar-proxy" title="Proxy listening address">
      <Icon name="forward" size={12} />{connection.proxyAddr}
    </span>
  {/if}

  <button
    type="button"
    class="switch"
    class:on
    role="switch"
    aria-checked={on}
    disabled={!intercept.loaded}
    data-testid="topbar-intercept"
    title={`Hold requests at the proxy (${toggleChord})`}
    onclick={() => void intercept.setEnabled(!intercept.enabled)}
  >
    <span class="track" aria-hidden="true"><span class="thumb"></span></span>
    <span class="sw-label">Intercept</span>
    {#if intercept.held.length > 0}<span class="held">{intercept.held.length}</span>{/if}
  </button>

  <span class="sep" aria-hidden="true"></span>

  <span
    class="ws"
    data-testid="ws-dot"
    data-state={events.connected ? 'connected' : 'disconnected'}
    role="img"
    aria-label={events.connected ? 'Live connection: connected' : 'Live connection: disconnected'}
    title={events.connected ? 'Live connection: connected' : 'Live connection: disconnected'}
  ></span>
  <IconButton
    icon="keyboard"
    label="Keyboard shortcuts"
    kbd="?"
    data-testid="shortcuts-button"
    onclick={onShortcuts}
  />
  <IconButton
    icon={appearance.theme === 'dark' ? 'sun' : 'moon'}
    label={appearance.theme === 'dark' ? 'Switch to light theme' : 'Switch to dark theme'}
    data-testid="theme-toggle"
    onclick={() => appearance.toggle()}
  />
</header>

<style>
  .top {
    grid-area: top;
    display: flex;
    align-items: center;
    gap: var(--s-2);
    min-width: 0;
    padding: 0 var(--s-2) 0 var(--s-4);
    background: var(--bg-elev);
    border-bottom: 1px solid var(--border);
    white-space: nowrap;
  }
  .brand {
    font-size: var(--fs-lg);
    font-weight: 650;
    letter-spacing: -0.01em;
  }
  .here {
    color: var(--fg-muted);
    margin-left: var(--s-1);
    padding-left: var(--s-3);
    border-left: 1px solid var(--border);
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .spacer {
    flex: 1 1 auto;
  }
  .sep {
    width: 1px;
    height: 20px;
    margin: 0 var(--s-1);
    background: var(--border);
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: var(--s-1);
    height: 22px;
    padding: 0 var(--s-2);
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    background: var(--bg-sunken);
    border: 1px solid var(--border);
    border-radius: var(--r-pill);
  }
  .switch {
    display: inline-flex;
    align-items: center;
    gap: var(--s-2);
    height: var(--control-h-md);
    min-height: 0;
    padding: 0 var(--s-2);
    font: inherit;
    font-size: var(--fs-sm);
    color: var(--fg-muted);
    background: transparent;
    border: 1px solid transparent;
    border-radius: var(--r-1);
    cursor: pointer;
  }
  .switch:hover:not(:disabled) {
    color: var(--fg);
    background: var(--bg-hover);
  }
  .switch:disabled {
    opacity: 0.45;
    cursor: default;
  }
  .track {
    position: relative;
    width: 26px;
    height: 14px;
    border-radius: var(--r-pill);
    background: var(--bg-active);
    border: 1px solid var(--border-strong);
    transition: background 120ms ease;
  }
  .thumb {
    position: absolute;
    top: 1px;
    left: 1px;
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: var(--fg-muted);
    transition: transform 120ms ease;
  }
  .switch.on {
    color: var(--warn);
  }
  .switch.on .track {
    background: var(--warn-bg);
    border-color: var(--warn);
  }
  .switch.on .thumb {
    transform: translateX(12px);
    background: var(--warn);
  }
  .held {
    min-width: 16px;
    height: 16px;
    padding: 0 4px;
    font-size: var(--fs-xs);
    line-height: 16px;
    text-align: center;
    color: var(--warn-fg);
    background: var(--warn-bg);
    border-radius: var(--r-pill);
  }
  .ws {
    width: 8px;
    height: 8px;
    margin: 0 var(--s-2);
    border-radius: 50%;
    background: var(--danger);
  }
  .ws[data-state='connected'] {
    background: var(--ok);
  }
</style>
