<script lang="ts">
  import { EditorView } from '@codemirror/view';
  import ByteEditor from '../editor/ByteEditor.svelte';
  import { onMount, tick } from 'svelte';
  import ConnSettings from './ConnSettings.svelte';
  import DiffView from './DiffView.svelte';
  import RepeaterResponse from './RepeaterResponse.svelte';
  import SendHistory from './SendHistory.svelte';
  import { hostCaret, repeater, type Mode, type RepeaterTab, type SendRecord } from '../stores/repeater.svelte';
  import { timeShort } from '../format';
  import { decoder } from '../stores/decoder.svelte';
  import { router } from '../router.svelte';
  import { assistedChips, type AssistedToggles } from '../editor/assisted';
  import { bytesToLatin1, latin1ToBytes } from '../editor/encoding';
  import { toasts } from '../ui/toasts.svelte';
  import { chordLabel } from '../copy-actions.svelte';
  import Button from '../ui/Button.svelte';
  import Segmented from '../ui/Segmented.svelte';
  import Splitter from '../ui/Splitter.svelte';
  import Toolbar from '../ui/Toolbar.svelte';

  // One repeater tab: the request (connection card, toolbar, editor) beside
  // the response. The tab object is reactive store state; its `raw` bytes are
  // bound two-way to the editable ByteEditor. `onsend` is the view's send
  // (it raises the error toast), shared with the Ctrl+Space chord.
  let { tab, onsend }: { tab: RepeaterTab; onsend: (id: string) => void } = $props();

  // Assisted toggles (both default on, per the UI contract). Display-only: the
  // server applies the real edits; the chips describe what it will do.
  let toggles = $state<AssistedToggles>({ contentLength: true, lineEndings: true });

  // The assisted chip surface is shown ONLY in assisted mode. In raw mode the
  // bytes go untouched, so no assisted chips are rendered above the editor.
  const chips = $derived(tab.mode === 'assisted' ? assistedChips(tab.raw, toggles) : []);

  const MODES = [
    { value: 'assisted' as const, label: 'Assisted', testid: 'mode-assisted' },
    { value: 'raw' as const, label: 'Raw', testid: 'mode-raw' },
  ];

  // The ByteEditor seeds its document from `bytes` once at mount and thereafter
  // flows edits OUT through the binding; it does not react to an external `bytes`
  // reset. So when we replace the whole buffer (paste-raw), bump this epoch to
  // force a remount that re-seeds from the new bytes. Ordinary keystroke edits
  // flow editor -> store and never bump it, so the caret is preserved.
  let editorEpoch = $state(0);
  let editorWrap = $state<HTMLElement>();

  // PASTE-RAW (first-class). The true byte-exact inverse of "Copy raw": read
  // the clipboard text and map it Latin-1 1:1 (every code point 0x00..0xFF ->
  // that byte), replacing the whole request buffer. Unlike the editor's
  // in-editor paste (UTF-8, wire-safe for human text), this preserves raw bytes
  // exactly — the path the paste-raw round-trip gate exercises.
  let pasteError = $state<string | null>(null);
  async function pasteRaw(): Promise<void> {
    pasteError = null;
    try {
      const text = (await navigator.clipboard?.readText()) ?? '';
      repeater.setRaw(tab.id, latin1ToBytes(text));
      editorEpoch += 1;
    } catch {
      pasteError = 'Paste failed: the clipboard is unavailable. Allow clipboard access and try again.';
    }
  }

  // COPY RAW: the whole draft as Latin-1 text (byte b -> U+00b), so pasting it
  // back with "Paste raw" restores every byte.
  async function copyRaw(): Promise<void> {
    try {
      await navigator.clipboard.writeText(bytesToLatin1(tab.raw));
      toasts.push({ kind: 'success', text: `Copied raw request (${tab.raw.length} bytes).` });
    } catch {
      toasts.push({ kind: 'error', text: 'Copy failed: the clipboard is unavailable.' });
    }
  }

  // DECODE SELECTION: the request editor's selection, read from the editor
  // state (byte-exact; the DOM text holds placeholder glyphs). An empty
  // selection sends the whole draft.
  function decodeSelection(): void {
    const cm = editorWrap?.querySelector('.cm-editor');
    const view = cm instanceof HTMLElement ? EditorView.findFromDOM(cm) : null;
    let bytes = tab.raw;
    if (view) {
      const { from, to } = view.state.selection.main;
      if (to > from) bytes = latin1ToBytes(view.state.sliceDoc(from, to));
    }
    decoder.seed(bytes, false);
    router.navigate('/decoder');
  }

  function editorView(): EditorView | null {
    const cm = editorWrap?.querySelector('.cm-editor');
    return cm instanceof HTMLElement ? EditorView.findFromDOM(cm) : null;
  }

  // A fresh template tab: put the caret at the end of its empty Host header.
  onMount(() => {
    if (!tab.focusHost) return;
    void tick().then(() => {
      const view = editorView();
      if (!view) return;
      const at = hostCaret(tab.raw);
      if (at !== null && at <= view.state.doc.length) view.dispatch({ selection: { anchor: at } });
      view.focus();
      repeater.hostFocused(tab.id);
    });
  });

  // ---- Send history and compare ----
  const history = $derived(tab.history ?? []);
  const shown = $derived(
    tab.outcome ? history.findIndex((r) => r.exchange_id === tab.outcome?.exchange_id) : -1,
  );
  const canCompare = $derived(repeater.comparePair(tab) !== null);

  type Compare =
    | { state: 'loading' }
    | { state: 'error'; message: string }
    | { state: 'ready'; older: string; newer: string; olderLabel: string; newerLabel: string };
  let compare = $state<Compare | null>(null);
  let compareSeq = 0;

  function sendLabel(r: SendRecord): string {
    const status = r.status !== undefined ? String(r.status) : 'status unknown';
    return `${timeShort(r.at)}  ${status}`;
  }

  async function toggleCompare(): Promise<void> {
    if (compare) {
      compare = null;
      return;
    }
    const seq = ++compareSeq;
    compare = { state: 'loading' };
    try {
      const c = await repeater.compareWithPrevious(tab.id);
      if (seq !== compareSeq) return;
      compare = c
        ? {
            state: 'ready',
            older: c.olderText,
            newer: c.newerText,
            olderLabel: sendLabel(c.older),
            newerLabel: sendLabel(c.newer),
          }
        : { state: 'error', message: 'Needs two sends with a response.' };
    } catch (e) {
      if (seq !== compareSeq) return;
      compare = { state: 'error', message: e instanceof Error ? e.message : 'could not load the responses' };
    }
  }

  function closeCompare(): void {
    compareSeq++;
    compare = null;
  }

  // A new send or a restore changes the shown response: leave compare mode.
  let lastShown: string | undefined;
  $effect(() => {
    const id = tab.outcome?.exchange_id;
    if (lastShown !== undefined && id !== lastShown) closeCompare();
    lastShown = id;
  });

  function restore(index: number): void {
    closeCompare();
    repeater.restore(tab.id, index);
  }

  // Panes sit side by side from 1280px; below that they stack.
  // Initialised synchronously so the first frame already has the right
  // direction; the effect follows later viewport changes.
  const STACK_QUERY = '(max-width: 1279px)';
  const canMatch = typeof window !== 'undefined' && typeof window.matchMedia === 'function';
  let stacked = $state(canMatch && window.matchMedia(STACK_QUERY).matches);
  $effect(() => {
    if (!canMatch) return;
    const mq = window.matchMedia(STACK_QUERY);
    stacked = mq.matches;
    const on = (e: MediaQueryListEvent) => (stacked = e.matches);
    mq.addEventListener('change', on);
    return () => mq.removeEventListener('change', on);
  });
</script>

<div class="tab-body" data-testid="repeater-tab-body">
  <SendHistory
    {history}
    {shown}
    dirty={tab.dirty}
    notice={tab.notice}
    {canCompare}
    comparing={compare !== null}
    onrestore={restore}
    onclear={() => {
      closeCompare();
      repeater.clearHistory(tab.id);
    }}
    oncompare={() => void toggleCompare()}
  />
  <div class="panes">
    <Splitter
      direction={stacked ? 'column' : 'row'}
      storageKey="repeater-split"
      min={[260, 220]}
      label="Resize request and response"
    >
      {#snippet a()}
        <section class="request" aria-label="Request">
          <ConnSettings conn={tab.conn} onChange={(patch) => repeater.setConn(tab.id, patch)} />

          <Toolbar label="Request actions" dense>
            <Button
              variant="primary"
              icon="send"
              kbd={chordLabel('send')}
              data-testid="send"
              disabled={tab.sending}
              onclick={() => onsend(tab.id)}>{tab.sending ? 'Sending…' : 'Send'}</Button
            >
            <span class="sep"></span>
            <Button size="sm" variant="ghost" data-testid="paste-raw" onclick={pasteRaw}>Paste raw</Button>
            <Button size="sm" variant="ghost" icon="copy" data-testid="copy-draft-raw" onclick={copyRaw}
              >Copy raw</Button
            >
            <Button
              size="sm"
              variant="ghost"
              icon="decoder"
              data-testid="decode-selection"
              title={`Decode selection (${chordLabel('decode-selection') ?? ''})`}
              onclick={decodeSelection}>Decode selection</Button
            >
            <span class="spacer"></span>
            <Segmented
              size="sm"
              label="Editor mode"
              options={MODES}
              value={tab.mode}
              onchange={(m: Mode) => repeater.setMode(tab.id, m)}
            />
          </Toolbar>

          {#if tab.mode === 'assisted'}
            <div class="assist-opts" role="group" aria-label="Assisted options">
              <span class="opts-label">On send</span>
              <label class="opt" class:on={toggles.contentLength}>
                <input type="checkbox" data-testid="toggle-content-length" bind:checked={toggles.contentLength} />
                <span>Fix Content-Length</span>
              </label>
              <label class="opt" class:on={toggles.lineEndings}>
                <input type="checkbox" data-testid="toggle-line-endings" bind:checked={toggles.lineEndings} />
                <span>Normalize line endings</span>
              </label>
            </div>
          {/if}

          {#if pasteError}
            <p class="warn" data-testid="paste-error" role="status">{pasteError}</p>
          {/if}
          {#if tab.saveError}
            <p class="warn" data-testid="tab-save-error" role="status">{tab.saveError}</p>
          {/if}

          <!-- REQUEST: editable byte editor bound to the tab's raw bytes. It stays
               the first CodeMirror editor in DOM order (the response is pane B). -->
          <div class="editor-wrap" data-testid="repeater-request" bind:this={editorWrap}>
            {#key `${tab.id}-${editorEpoch}-${tab.rev ?? 0}`}
              <ByteEditor
                bind:bytes={() => tab.raw, (v) => repeater.setRaw(tab.id, v)}
                readonly={false}
                showCopyRaw={false}
                {chips}
              />
            {/key}
          </div>
        </section>
      {/snippet}
      {#snippet b()}
        {#if compare?.state === 'ready'}
          <DiffView
            olderText={compare.older}
            newerText={compare.newer}
            olderLabel={compare.olderLabel}
            newerLabel={compare.newerLabel}
            onclose={closeCompare}
          />
        {:else if compare}
          <div class="compare-status">
            {#if compare.state === 'loading'}
              <p class="note" data-testid="compare-loading">Loading both responses…</p>
            {:else}
              <p class="warn" data-testid="compare-error" role="alert">Compare failed: {compare.message}</p>
            {/if}
            <Button size="sm" variant="ghost" icon="close" onclick={closeCompare}>Close</Button>
          </div>
        {:else}
          <RepeaterResponse {tab} />
        {/if}
      {/snippet}
    </Splitter>
  </div>
</div>

<style>
  .tab-body {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
  }
  .panes {
    flex: 1;
    min-height: 0;
    min-width: 0;
  }
  .compare-status {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    padding: var(--s-3);
  }
  .note {
    margin: 0;
    font-size: var(--fs-sm);
    color: var(--fg-muted);
  }
  .request {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
    height: 100%;
    min-height: 0;
    padding: var(--s-2) var(--s-3) var(--s-2) 0;
  }
  .assist-opts {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-1) var(--s-2);
    font-size: var(--fs-sm);
  }
  .opts-label {
    color: var(--fg-muted);
  }
  .opt {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 22px;
    padding: 0 8px 0 6px;
    border: 1px solid var(--border);
    border-radius: var(--r-pill);
    background: var(--bg-sunken);
    color: var(--fg-muted);
    cursor: pointer;
  }
  .opt.on {
    border-color: var(--border-strong);
    background: var(--chip-bg);
    color: var(--chip-fg);
  }
  .opt input {
    margin: 0;
  }
  .opt:focus-within {
    outline: 2px solid var(--focus);
    outline-offset: 1px;
  }
  .editor-wrap {
    flex: 1;
    min-height: 120px;
    min-width: 0;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: var(--r-2);
    background: var(--bg-sunken);
  }
  .editor-wrap > :global(.byte-editor) {
    min-height: 100%;
  }
  .editor-wrap :global(.cm-host) {
    flex: 1;
    display: flex;
    flex-direction: column;
  }
  .editor-wrap :global(.cm-editor) {
    flex: 1;
  }
  .warn {
    margin: 0;
    font-size: var(--fs-sm);
    color: var(--warn);
  }
</style>
