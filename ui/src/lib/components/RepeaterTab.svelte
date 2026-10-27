<script lang="ts">
  import ByteEditor from '../editor/ByteEditor.svelte';
  import ConnSettings from './ConnSettings.svelte';
  import CopyMenu from './CopyMenu.svelte';
  import ResultHeader from './ResultHeader.svelte';
  import { repeater, type RepeaterTab } from '../stores/repeater.svelte';
  import { assistedChips, type AssistedToggles } from '../editor/assisted';
  import { latin1ToBytes } from '../editor/encoding';
  import { apiGetBytes } from '../api/client';
  import { ApiError } from '../api/errors';

  // One repeater tab's editor + connection + result. The tab object is reactive
  // store state; its `raw` bytes are bound two-way to the editable ByteEditor.
  let { tab }: { tab: RepeaterTab } = $props();

  // Assisted toggles (both default on, per the UI contract). Display-only: the
  // server applies the real edits; the chips describe what it will do.
  let toggles = $state<AssistedToggles>({ contentLength: true, lineEndings: true });

  // The assisted chip surface is shown ONLY in assisted mode. In raw mode the
  // bytes go untouched, so no assisted chips are rendered above the editor.
  const chips = $derived(tab.mode === 'assisted' ? assistedChips(tab.raw, toggles) : []);

  // The ByteEditor seeds its document from `bytes` once at mount and thereafter
  // flows edits OUT through the binding; it does not react to an external `bytes`
  // reset. So when we replace the whole buffer (paste-raw), bump this epoch to
  // force a remount that re-seeds from the new bytes. Ordinary keystroke edits
  // flow editor -> store and never bump it, so the caret is preserved.
  let editorEpoch = $state(0);

  // PASTE-RAW (first-class). The true byte-exact inverse of the editor's "Copy
  // raw": read the clipboard text and map it Latin-1 1:1 (every code point
  // 0x00..0xFF -> that byte), replacing the whole request buffer. Unlike the
  // editor's in-editor paste (UTF-8, wire-safe for human text), this preserves
  // raw bytes exactly — the path the paste-raw round-trip gate exercises.
  let pasteError = $state<string | null>(null);
  async function pasteRaw(): Promise<void> {
    pasteError = null;
    try {
      const text = (await navigator.clipboard?.readText()) ?? '';
      repeater.setRaw(tab.id, latin1ToBytes(text));
      editorEpoch += 1;
    } catch {
      pasteError = 'Paste failed: clipboard unavailable.';
    }
  }

  // ---- Response bytes -------------------------------------------------------
  // Sourced via apiGetBytes('/exchanges/{id}/body/resp') for the outcome's
  // exchange_id (the send response carries only the id, not the bytes). A 404
  // means the response had no body.
  interface RespState {
    bytes: Uint8Array;
    absent: boolean;
    error: string | null;
  }
  let resp = $state<RespState | null>(null);

  $effect(() => {
    const id = tab.outcome?.exchange_id;
    resp = null;
    if (!id) return;
    const current = id;
    void (async () => {
      try {
        const { bytes } = await apiGetBytes(
          `/api/v1/exchanges/${encodeURIComponent(current)}/body/resp`,
        );
        if (tab.outcome?.exchange_id === current) {
          resp = { bytes, absent: false, error: null };
        }
      } catch (e) {
        if (tab.outcome?.exchange_id !== current) return;
        if (e instanceof ApiError && e.status === 404) {
          resp = { bytes: new Uint8Array(), absent: true, error: null };
        } else {
          resp = {
            bytes: new Uint8Array(),
            absent: false,
            error: e instanceof Error ? e.message : 'response body read failed',
          };
        }
      }
    })();
  });

  function send(): void {
    void repeater.send(tab.id);
  }
</script>

<div class="tab-body" data-testid="repeater-tab-body">
  <ConnSettings conn={tab.conn} onChange={(patch) => repeater.setConn(tab.id, patch)} />

  <div class="toolbar">
    <div class="modes" role="group" aria-label="editor mode">
      <button
        type="button"
        data-testid="mode-assisted"
        class:active={tab.mode === 'assisted'}
        aria-pressed={tab.mode === 'assisted'}
        onclick={() => repeater.setMode(tab.id, 'assisted')}>Assisted</button
      >
      <button
        type="button"
        data-testid="mode-raw"
        class:active={tab.mode === 'raw'}
        aria-pressed={tab.mode === 'raw'}
        onclick={() => repeater.setMode(tab.id, 'raw')}>Raw</button
      >
    </div>

    {#if tab.mode === 'assisted'}
      <div class="toggles" role="group" aria-label="assisted toggles">
        <label class="check">
          <input type="checkbox" data-testid="toggle-content-length" bind:checked={toggles.contentLength} />
          <span>Content-Length</span>
        </label>
        <label class="check">
          <input type="checkbox" data-testid="toggle-line-endings" bind:checked={toggles.lineEndings} />
          <span>Line endings</span>
        </label>
      </div>
    {/if}

    <div class="spacer"></div>

    <button type="button" data-testid="paste-raw" onclick={pasteRaw}>Paste raw</button>
    <button type="button" data-testid="send" class="send" disabled={tab.sending} onclick={send}>
      {tab.sending ? 'Sending…' : 'Send'}
    </button>
  </div>

  {#if pasteError}
    <p class="warn" data-testid="paste-error" role="status">{pasteError}</p>
  {/if}

  <!-- REQUEST: editable byte editor bound to the tab's raw bytes. -->
  <div class="editor-wrap" data-testid="repeater-request">
    {#key `${tab.id}-${editorEpoch}`}
      <ByteEditor
        bind:bytes={() => tab.raw, (v) => repeater.setRaw(tab.id, v)}
        readonly={false}
        {chips}
      />
    {/key}
  </div>

  <!-- Distinct error channels: send vs draft-save. -->
  {#if tab.error}
    <p class="error" data-testid="tab-error" role="alert">Send failed: {tab.error}</p>
  {/if}
  {#if tab.saveError}
    <p class="warn" data-testid="tab-save-error" role="status">{tab.saveError}</p>
  {/if}

  <!-- RESULT -->
  {#if tab.outcome}
    <ResultHeader outcome={tab.outcome} />
    <CopyMenu id={tab.outcome.exchange_id} />

    <div class="pane" data-testid="repeater-response">
      <h4>Response</h4>
      {#if resp === null}
        <p class="note">Loading response…</p>
      {:else if resp.error}
        <p class="error" role="alert" data-testid="resp-error">{resp.error}</p>
      {:else if resp.absent}
        <p class="note" data-testid="resp-empty">No response body.</p>
      {:else}
        {#key `${tab.outcome.exchange_id}-${resp.bytes.length}`}
          <div data-testid="result-resp-body">
            <ByteEditor bytes={resp.bytes} readonly={true} />
          </div>
        {/key}
      {/if}
    </div>
  {/if}
</div>

<style>
  .tab-body {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    min-width: 0;
  }
  .toolbar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.5rem;
  }
  .spacer {
    flex: 1 1 auto;
  }
  .modes {
    display: inline-flex;
    border: 1px solid #3a3a4a;
    border-radius: 4px;
    overflow: hidden;
  }
  .modes button {
    background: #22222e;
    color: #cfd0ff;
    border: none;
    padding: 0.25rem 0.6rem;
    font-size: 0.78rem;
    cursor: pointer;
  }
  .modes button.active {
    background: #3a3a5a;
  }
  .toggles {
    display: flex;
    gap: 0.6rem;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 0.3rem;
    font-size: 0.76rem;
  }
  .toolbar button:not(.modes button) {
    font-size: 0.78rem;
    padding: 0.25rem 0.6rem;
    border: 1px solid #3a3a4a;
    border-radius: 4px;
    background: #22222e;
    color: #cfd0ff;
    cursor: pointer;
  }
  .toolbar button.send {
    background: #2a4a2a;
    color: #cfeccf;
  }
  .toolbar button:disabled {
    opacity: 0.6;
    cursor: default;
  }
  .editor-wrap {
    min-width: 0;
    border: 1px solid #8883;
    border-radius: 4px;
  }
  .pane {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
  }
  h4 {
    margin: 0.2rem 0 0;
    font-size: 0.8rem;
    opacity: 0.8;
  }
  .note {
    font-size: 0.8rem;
    opacity: 0.7;
    margin: 0;
  }
  .error {
    font-size: 0.85rem;
    color: #ff9b9b;
    margin: 0;
  }
  .warn {
    font-size: 0.8rem;
    color: #ffd9a0;
    margin: 0;
  }
</style>
