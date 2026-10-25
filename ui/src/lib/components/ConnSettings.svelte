<script lang="ts">
  import type { TabConn } from '../stores/repeater.svelte';

  // Per-tab connection settings. The parent owns the tab's `conn`; every change
  // is pushed back through `onChange` so the store can persist the draft. SNI is
  // DISTINCT from the request's Host header: it is the TLS server name sent in
  // the handshake, so leaving it blank lets the server default it.
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
</script>

<div class="conn" data-testid="conn-settings">
  <label class="field">
    <span>Host</span>
    <input
      data-testid="conn-host"
      type="text"
      value={conn.host}
      oninput={(e) => onChange({ host: e.currentTarget.value })}
      placeholder="host or IP"
    />
  </label>

  <label class="field port">
    <span>Port</span>
    <input
      data-testid="conn-port"
      type="number"
      min="1"
      max="65535"
      value={conn.port}
      oninput={(e) => onChange({ port: Number(e.currentTarget.value) })}
    />
  </label>

  <label class="check">
    <input
      data-testid="conn-tls"
      type="checkbox"
      checked={conn.tls}
      onchange={(e) => onChange({ tls: e.currentTarget.checked })}
    />
    <span>TLS</span>
  </label>

  <label class="field">
    <span>SNI</span>
    <input
      data-testid="conn-sni"
      type="text"
      value={conn.sni ?? ''}
      oninput={(e) => onChange({ sni: e.currentTarget.value })}
      placeholder="(defaults to host)"
      disabled={!conn.tls}
    />
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

  <label class="field">
    <span>Follow redirects</span>
    <select
      data-testid="conn-follow"
      value={conn.follow}
      onchange={(e) => onChange({ follow: e.currentTarget.value as TabConn['follow'] })}
    >
      <option value="off">off</option>
      <option value="in-scope">in-scope</option>
      <option value="always">always</option>
    </select>
  </label>
</div>

<style>
  .conn {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.5rem 0.75rem;
    padding: 0.4rem 0;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
    font-size: 0.72rem;
    opacity: 0.9;
  }
  .field input,
  .field select {
    padding: 0.2rem 0.35rem;
    font-size: 0.8rem;
  }
  .field.port input {
    width: 5rem;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 0.3rem;
    font-size: 0.78rem;
  }
</style>
