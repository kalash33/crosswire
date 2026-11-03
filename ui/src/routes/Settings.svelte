<script lang="ts">
  // /settings (U1 D6): appearance, shortcut reference, connection details and
  // build info. Server-reported values are text only; the only links are
  // literal constants.
  import { onDestroy, onMount } from 'svelte';
  import { appearance } from '../lib/stores/appearance.svelte';
  import { connection } from '../lib/stores/connection.svelte';
  import {
    keymap,
    formatChord,
    recordChord,
    ACTION_LABELS,
    SCOPES,
    SCOPE_LABELS,
    type Action,
  } from '../lib/keymap';
  import { caPageUrl } from '../lib/components/start';
  import Segmented from '../lib/ui/Segmented.svelte';
  import Kbd from '../lib/ui/Kbd.svelte';
  import Button from '../lib/ui/Button.svelte';
  import Badge from '../lib/ui/Badge.svelte';
  import CopyValue from '../lib/components/CopyValue.svelte';
  import type { Theme } from '../lib/theme';

  const THEMES = [
    { value: 'dark' as Theme, label: 'Dark', icon: 'moon' as const, testid: 'theme-dark' },
    { value: 'light' as Theme, label: 'Light', icon: 'sun' as const, testid: 'theme-light' },
  ];
  // The keymap is plain TS; a version counter makes the table re-read it.
  let version = $state(0);
  const bindings = $derived.by(() => {
    void version;
    return keymap.bindings();
  });
  const groups = $derived(
    SCOPES.map((scope) => ({
      scope,
      label: SCOPE_LABELS[scope],
      rows: bindings.filter((b) => b.scope === scope),
    })).filter((g) => g.rows.length > 0),
  );
  const anyOverridden = $derived(bindings.some((b) => keymap.isOverridden(b.action)));
  const unsubscribe = keymap.subscribe(() => version++);
  onDestroy(unsubscribe);

  let recording = $state<Action | null>(null);
  let refusal = $state<{ action: Action; reason: string } | null>(null);

  function startRecording(action: Action): void {
    recording = action;
    refusal = null;
  }

  function onRecordKey(e: KeyboardEvent, action: Action): void {
    const r = recordChord(e, action);
    if (r.kind === 'ignore') return;
    // Own the keystroke so it neither types nor triggers a browser command.
    e.preventDefault();
    e.stopPropagation();
    if (r.kind === 'cancel') {
      recording = null;
      refusal = null;
    } else if (r.kind === 'refused') {
      refusal = { action, reason: r.reason };
    } else {
      keymap.remap(action, r.chord);
      recording = null;
      refusal = null;
    }
  }

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
      {#each groups as g (g.scope)}
      <section class="group" data-testid="shortcut-group-{g.scope}">
      <h3>{g.label}</h3>
      <table>
        <tbody>
          {#each g.rows as b (b.action)}
            <tr>
              <td>
                {b.label}
                {#if keymap.isOverridden(b.action)}
                  <Badge value="custom" />
                {/if}
                {#if keymap.shadows(b.action).length > 0}
                  <div class="hint" data-testid="shortcut-shadow-{b.action}">
                    In this view this hides {keymap.shadows(b.action).map((o) => `"${ACTION_LABELS[o]}"`).join(', ')}
                    on the same shortcut. Change one of them to use both.
                  </div>
                {/if}
                {#if recording === b.action}
                  <div class="hint" role="status">Press the new shortcut… Esc cancels</div>
                {/if}
                {#if refusal && refusal.action === b.action}
                  <div class="hint refusal" role="alert" data-testid="shortcut-error-{b.action}">{refusal.reason}</div>
                {/if}
              </td>
              <td class="chord">
                {#if b.chord}<Kbd keys={formatChord(b.chord)} />{:else}<span class="unset">Not set</span>{/if}
                <span class="actions">
                  <Button
                    size="sm"
                    variant="ghost"
                    data-testid="shortcut-change-{b.action}"
                    aria-label="Change shortcut for {b.label}"
                    onclick={() => startRecording(b.action)}
                    onkeydown={(e: KeyboardEvent) => {
                      if (recording === b.action) onRecordKey(e, b.action);
                    }}
                    onblur={() => {
                      if (recording === b.action) {
                        recording = null;
                        refusal = null;
                      }
                    }}
                  >
                    {recording === b.action ? 'Recording…' : 'Change'}
                  </Button>
                  {#if keymap.isOverridden(b.action)}
                    <Button
                      size="sm"
                      variant="ghost"
                      data-testid="shortcut-reset-{b.action}"
                      aria-label="Reset shortcut for {b.label}"
                      onclick={() => keymap.reset(b.action)}
                    >
                      Reset
                    </Button>
                  {/if}
                </span>
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
      </section>
      {/each}
      {#if anyOverridden}
        <p class="reset-all">
          <Button size="sm" data-testid="shortcuts-reset-all" onclick={() => keymap.reset()}>Reset all</Button>
        </p>
      {/if}
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
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: var(--s-2);
    text-align: right;
  }
  h3 {
    margin: var(--s-3) 0 var(--s-1);
    font-size: var(--fs-sm);
    font-weight: 600;
    color: var(--fg-muted);
  }
  .group:first-of-type h3 {
    margin-top: 0;
  }
  .unset {
    color: var(--fg-muted);
  }
  .actions {
    display: inline-flex;
    gap: var(--s-1);
  }
  .refusal {
    color: var(--danger);
  }
  .reset-all {
    margin: var(--s-3) 0 0;
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
