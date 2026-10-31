<script lang="ts">
  import type { TabConn } from '../stores/repeater.svelte';
  import Icon from '../ui/Icon.svelte';
  import Input from '../ui/Input.svelte';
  import Select from '../ui/Select.svelte';

  // Per-tab connection settings, in a collapsible card above the request
  // editor. The parent owns the tab's `conn`; every change is pushed back
  // through `onChange` so the store can persist the draft. SNI is DISTINCT
  // from the request's Host header: it is the TLS server name sent in the
  // handshake, so leaving it blank lets the server default it.
  let {
    conn,
    onChange,
  }: {
    conn: TabConn;
    onChange: (patch: Partial<TabConn>) => void;
  } = $props();

  // `verify` is a string on the wire (`web-pki` default / `accept-invalid`);
  // model it as a boolean "verify certificate" checkbox for the UI.
  const verifyOn = $derived(conn.verify !== 'accept-invalid');

  // Collapsed state is a per-browser convenience shared by every tab. Storage
  // can be unavailable (private mode, blocked site data): never fatal.
  const KEY = 'crosswire.repeater.conn-collapsed';
  function loadCollapsed(): boolean {
    try {
      return localStorage.getItem(KEY) === '1';
    } catch {
      return false;
    }
  }
  let collapsed = $state(loadCollapsed());
  function toggle(): void {
    collapsed = !collapsed;
    try {
      localStorage.setItem(KEY, collapsed ? '1' : '0');
    } catch {
      // storage unavailable: the state just does not persist
    }
  }

  // The one-line target shown in the card header (text only).
  const target = $derived(conn.host.trim() ? `${conn.host.trim()}:${conn.port}` : 'No target set');
  const uid = $props.id();
  const bodyId = `conn-body-${uid}`;
</script>

<section class="conn-card" class:collapsed data-testid="conn-settings">
  <button
    type="button"
    class="conn-head"
    aria-expanded={!collapsed}
    aria-controls={bodyId}
    data-testid="conn-toggle"
    onclick={toggle}
  >
    <span class="chev"><Icon name={collapsed ? 'chevron-right' : 'chevron-down'} size={14} /></span>
    <span class="head-label">Target</span>
    <span class="head-target" class:unset={!conn.host.trim()}>{target}</span>
    {#if conn.tls}<span class="flag">TLS</span>{/if}
    {#if conn.tls && !verifyOn}<span class="flag warn">no verify</span>{/if}
    {#if conn.reuse}<span class="flag">reuse</span>{/if}
    {#if conn.follow !== 'off'}<span class="flag">follow {conn.follow}</span>{/if}
  </button>

  {#if !collapsed}
    <div class="conn-body" id={bodyId}>
      <label class="field host">
        <span>Host</span>
        <Input
          size="sm"
          mono
          data-testid="conn-host"
          type="text"
          value={conn.host}
          oninput={(e) => onChange({ host: e.currentTarget.value })}
          placeholder="host or IP"
          spellcheck="false"
          autocomplete="off"
        />
      </label>

      <label class="field port">
        <span>Port</span>
        <Input
          size="sm"
          mono
          data-testid="conn-port"
          type="number"
          min="1"
          max="65535"
          value={conn.port}
          oninput={(e) => onChange({ port: Number(e.currentTarget.value) })}
        />
      </label>

      <label class="field sni">
        <span>SNI</span>
        <Input
          size="sm"
          mono
          data-testid="conn-sni"
          type="text"
          value={conn.sni ?? ''}
          oninput={(e) => onChange({ sni: e.currentTarget.value })}
          placeholder="defaults to host"
          disabled={!conn.tls}
          spellcheck="false"
          autocomplete="off"
        />
      </label>

      <label class="field follow">
        <span>Follow redirects</span>
        <Select
          size="sm"
          data-testid="conn-follow"
          value={conn.follow}
          onchange={(e) => onChange({ follow: e.currentTarget.value as TabConn['follow'] })}
        >
          <option value="off">Off</option>
          <option value="in-scope">In scope</option>
          <option value="always">Always</option>
        </Select>
      </label>

      <div class="checks" role="group" aria-label="Connection options">
        <label class="check">
          <input
            data-testid="conn-tls"
            type="checkbox"
            checked={conn.tls}
            onchange={(e) => onChange({ tls: e.currentTarget.checked })}
          />
          <span>TLS</span>
        </label>
        <label class="check">
          <input
            data-testid="conn-verify"
            type="checkbox"
            checked={verifyOn}
            onchange={(e) => onChange({ verify: e.currentTarget.checked ? 'web-pki' : 'accept-invalid' })}
            disabled={!conn.tls}
          />
          <span>Verify cert</span>
        </label>
        <label class="check">
          <input
            data-testid="conn-reuse"
            type="checkbox"
            checked={conn.reuse}
            onchange={(e) => onChange({ reuse: e.currentTarget.checked })}
          />
          <span>Reuse connection</span>
        </label>
      </div>
    </div>
  {/if}
</section>

<style>
  .conn-card {
    flex: none;
    border: 1px solid var(--border);
    border-radius: var(--r-2);
    background: var(--bg-elev);
  }
  .conn-head {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    width: 100%;
    min-width: 0;
    height: 32px;
    padding: 0 var(--s-2);
    font: inherit;
    font-size: var(--fs-sm);
    color: var(--fg);
    background: none;
    border: none;
    border-radius: var(--r-2);
    cursor: pointer;
    text-align: left;
  }
  .conn-head:hover {
    background: var(--bg-hover);
  }
  .conn-head:focus-visible {
    outline: 2px solid var(--focus);
    outline-offset: -2px;
  }
  .chev {
    display: inline-flex;
    color: var(--fg-muted);
  }
  .head-label {
    color: var(--fg-muted);
  }
  .head-target {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--font-mono);
    font-weight: 600;
  }
  .head-target.unset {
    font-family: var(--font-ui);
    font-weight: 400;
    color: var(--fg-faint);
  }
  .flag {
    flex: none;
    padding: 1px 6px;
    font-size: var(--fs-xs);
    color: var(--chip-fg);
    background: var(--chip-bg);
    border-radius: var(--r-1);
  }
  .flag.warn {
    color: var(--warn-fg);
    background: var(--warn-bg);
  }
  .conn-body {
    display: grid;
    grid-template-columns: minmax(10rem, 2fr) 6rem minmax(8rem, 1.5fr) minmax(8rem, 1fr);
    align-items: end;
    gap: var(--s-2) var(--s-3);
    padding: var(--s-1) var(--s-3) var(--s-3);
    border-top: 1px solid var(--border-faint);
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
    font-size: var(--fs-xs);
    color: var(--fg-muted);
  }
  .checks {
    grid-column: 1 / -1;
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-1) var(--s-4);
  }
  .check {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-sm);
    color: var(--fg);
    cursor: pointer;
  }
  @media (max-width: 720px) {
    .conn-body {
      grid-template-columns: 1fr 6rem;
    }
  }
</style>
