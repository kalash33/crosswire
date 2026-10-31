<script lang="ts">
  // /settings (U1 D6): appearance, shortcut reference, connection details and
  // build info. Server-reported values are text only; the only links are
  // literal constants.
  import { onMount } from 'svelte';
  import { appearance } from '../lib/stores/appearance.svelte';
  import { connection } from '../lib/stores/connection.svelte';
  import { keymap, formatChord } from '../lib/keymap';
  import { caPageUrl } from '../lib/components/start';
  import Segmented from '../lib/ui/Segmented.svelte';
  import Kbd from '../lib/ui/Kbd.svelte';
  import CopyValue from '../lib/components/CopyValue.svelte';
  import type { Theme } from '../lib/theme';

  const THEMES = [
    { value: 'dark' as Theme, label: 'Dark', icon: 'moon' as const, testid: 'theme-dark' },
    { value: 'light' as Theme, label: 'Light', icon: 'sun' as const, testid: 'theme-light' },
  ];
  const bindings = keymap.bindings();

  onMount(() => {
    void connection.load();
  });
</script>

<div class="settings" data-testid="settings-view">
  <h1>Settings</h1>
  <div class="col">
    <section class="card" aria-labelledby="s-appearance">
      <h2 id="s-appearance">Appearance</h2>
      <div class="field">
        <span class="name">Theme</span>
        <Segmented
          options={THEMES}
          value={appearance.theme}
          onchange={(t) => appearance.set(t)}
          label="Theme"
          size="sm"
        />
        <span class="hint">Saved in this browser.</span>
      </div>
    </section>

    <section class="card" aria-labelledby="s-shortcuts" data-testid="settings-shortcuts">
      <h2 id="s-shortcuts">Keyboard shortcuts</h2>
      <table>
        <tbody>
          {#each bindings as b (b.action)}
            <tr>
              <td>{b.label}</td>
              <td class="chord"><Kbd keys={formatChord(b.chord)} /></td>
            </tr>
          {/each}
        </tbody>
      </table>
    </section>
  </div>

  <div class="col">
    <section class="card" aria-labelledby="s-connection" data-testid="settings-connection">
      <h2 id="s-connection">Connection</h2>
      <dl>
        <dt>API</dt>
        <dd><CopyValue value={connection.apiOrigin} what="the API address" testid="settings-api" /></dd>
        <dt>Proxy</dt>
        <dd>
          <CopyValue value={connection.proxyAddrOrDefault} what="the proxy address" testid="settings-proxy" />
          {#if !connection.proxyAddr}
            <span class="hint">The server does not report its proxy address; this is the <code>crosswire serve</code> default.</span>
          {/if}
        </dd>
        <dt>CA page</dt>
        <dd>
          <CopyValue value={caPageUrl(connection.proxyAddrOrDefault)} what="the CA page address" testid="settings-ca" />
          <span class="hint">Serves <code>ca.der</code> and <code>ca.pem</code>.</span>
        </dd>
      </dl>
    </section>

    <section class="card" aria-labelledby="s-about" data-testid="settings-about">
      <h2 id="s-about">About</h2>
      {#if connection.health}
        <dl>
          {#if connection.health.version}
            <dt>Version</dt>
            <dd class="mono" data-testid="about-version">{connection.health.version}</dd>
          {/if}
          <dt>Schema version</dt>
          <dd class="mono" data-testid="about-schema-version">{connection.health.schemaVersion}</dd>
          <dt>Project format</dt>
          <dd class="mono" data-testid="about-format-version">{connection.health.formatVersion}</dd>
        </dl>
      {:else if connection.error}
        <p class="hint" role="alert">Could not read server details: {connection.error}</p>
      {:else}
        <p class="hint">Loading…</p>
      {/if}
      <p class="links">
        <a href="https://github.com/kalash33/crosswire" target="_blank" rel="noopener noreferrer">Source on GitHub</a>
        <a href="https://github.com/kalash33/crosswire/issues" target="_blank" rel="noopener noreferrer">Report an issue</a>
      </p>
    </section>
  </div>
</div>

<style>
  .settings {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: var(--s-4);
    align-items: start;
    max-width: 64rem;
    padding-bottom: var(--s-4);
  }
  h1 {
    grid-column: 1 / -1;
    margin: var(--s-1) 0 0;
    font-size: var(--fs-xl);
    font-weight: 600;
  }
  /* Two independent column stacks, so a short card never leaves a gap. */
  .col {
    display: flex;
    flex-direction: column;
    gap: var(--s-4);
    min-width: 0;
  }
  @media (max-width: 900px) {
    .settings {
      grid-template-columns: minmax(0, 1fr);
    }
  }
  .card {
    min-width: 0;
    padding: var(--s-4);
    background: var(--bg-elev);
    border: 1px solid var(--border);
    border-radius: var(--r-2);
  }
  h2 {
    margin: 0 0 var(--s-3);
    font-size: var(--fs-md);
    font-weight: 600;
  }
  .field {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--s-3);
  }
  .name {
    min-width: 6rem;
    color: var(--fg-muted);
  }
  .hint {
    font-size: var(--fs-sm);
    color: var(--fg-muted);
  }
  dl {
    display: grid;
    grid-template-columns: 7rem minmax(0, 1fr);
    gap: var(--s-2) var(--s-3);
    margin: 0;
    align-items: start;
  }
  dt {
    padding-top: 5px;
    color: var(--fg-muted);
  }
  dd {
    display: flex;
    flex-direction: column;
    gap: var(--s-1);
    min-width: 0;
    margin: 0;
  }
  dd.mono {
    padding-top: 5px;
    font-family: var(--font-mono);
  }
  code {
    font-family: var(--font-mono);
  }
  table {
    width: 100%;
    border-collapse: collapse;
  }
  td {
    padding: 5px 0;
    border-bottom: 1px solid var(--border-faint);
  }
  tr:last-child td {
    border-bottom: none;
  }
  .chord {
    text-align: right;
  }
  .links {
    display: flex;
    gap: var(--s-4);
    margin: var(--s-4) 0 0;
  }
  .links a {
    color: var(--accent);
  }
</style>
