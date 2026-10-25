<script lang="ts">
  import { onMount } from 'svelte';
  import { EditorState } from '@codemirror/state';
  import { EditorView } from '@codemirror/view';
  import { bytesToLatin1, latin1ToBytes, encodePastedText } from './encoding';
  import { nonPrintablePlaceholders } from './nonprintable';
  import { readCspNonce } from './csp';
  import type { Chip } from './chips';

  // `bytes` is bindable. In read-only use (Task 6 / history detail) it is set
  // once from the document after mount and never changes. In editable use
  // (Plan 5b: repeater / intercept) every document change pushes the edited
  // bytes back through this prop via the update listener below.
  //
  // `readonly` is true in the history detail and false for request editing.
  // `diagnostic` surfaces the partial/timed-out banner (§7.2). `chips` is the
  // optional assisted/result chip surface shown above the editor; the history
  // detail passes none, so its rendering is unchanged.
  let {
    bytes = $bindable(new Uint8Array()),
    readonly = true,
    diagnostic = null,
    chips = [],
  }: {
    bytes?: Uint8Array;
    readonly?: boolean;
    diagnostic?: string | null;
    chips?: Chip[];
  } = $props();

  let host: HTMLDivElement;
  let view: EditorView | undefined;
  let pasteChip = $state<string | null>(null);

  // PASTE-RAW. Normal CodeMirror paste would insert the clipboard text at the
  // cursor straight into the Latin-1 document, which corrupts any character
  // above U+00FF (latin1ToBytes masks it with & 0xFF). So in editable mode we
  // intercept paste, route the text through encodePastedText (UTF-8 — wire-safe)
  // and REPLACE THE WHOLE BUFFER with the result; non-Latin-1 input raises the
  // chip so the user knows it was re-encoded rather than mapped 1:1. In
  // read-only mode paste is inert.
  function onPaste(ev: ClipboardEvent): boolean {
    if (readonly || !view) return false;
    ev.preventDefault();
    const text = ev.clipboardData?.getData('text') ?? '';
    const { bytes: pasted, nonLatin1 } = encodePastedText(text);
    pasteChip = nonLatin1 ? 'Pasted non-Latin-1 text was UTF-8 encoded.' : null;
    view.dispatch({
      changes: { from: 0, to: view.state.doc.length, insert: bytesToLatin1(pasted) },
    });
    return true;
  }

  // COPY-RAW. Copies the ENTIRE buffer as a Latin-1 string (byte b -> U+00b),
  // the exact text the editor holds, so round-tripping through another tool
  // preserves every byte. Clipboard access can be unavailable (no permission /
  // insecure context); failure is non-fatal.
  async function copyRaw(): Promise<void> {
    try {
      await navigator.clipboard?.writeText(bytesToLatin1(bytes));
    } catch {
      // Clipboard unavailable (permissions / insecure context): nothing to do.
      pasteChip = 'Copy failed: clipboard unavailable.';
    }
  }

  onMount(() => {
    const nonce = readCspNonce();
    const extensions = [
      // BYTE FIDELITY: pin the line separator to "\n" so CodeMirror performs NO
      // line-ending normalization. With the default (undefined) facet, CM splits
      // on \n, \r\n and \r and rejoins every line with \n — which rewrites a
      // lone \r and a \r\n to \n and loses bytes. Pinned to "\n", CM splits and
      // joins only on \n, leaving \r (and \r\n) as literal bytes in the text, so
      // doc.toString() is an exact inverse of the input string. This holds in
      // BOTH directions: a typed/pasted edit re-reads the same way.
      EditorState.lineSeparator.of('\n'),
      EditorState.readOnly.of(readonly),
      EditorView.editable.of(!readonly),
      EditorView.lineWrapping,
      // PLACEHOLDER INTERACTION: non-printables are drawn as view-only replace
      // decorations; the REAL byte always lives in the document. The decoration
      // set is recomputed on every docChange, so editing text around a
      // placeholder only shifts positions — it never rewrites the byte the
      // placeholder stands for. Deleting a placeholder deletes its real byte
      // (the 1-char range), which is the intended edit.
      nonPrintablePlaceholders,
      // EDIT READ-BACK: reflect the editor's ACTUAL document back into `bytes` on
      // every change. doc.toString() -> latin1ToBytes travels the real CM
      // document, so the bound value is always byte-exact with what is shown.
      // In read-only mode the doc never changes, so this is inert there.
      EditorView.updateListener.of((u) => {
        if (u.docChanged) bytes = latin1ToBytes(u.state.doc.toString());
      }),
      EditorView.domEventHandlers({ paste: onPaste }),
      // Give CM's runtime <style> the CSP nonce when the server provided one;
      // omit the facet entirely otherwise (dev/preview has no server CSP).
      ...(nonce ? [EditorView.cspNonce.of(nonce)] : []),
    ];

    const state = EditorState.create({
      doc: bytesToLatin1(bytes),
      extensions,
    });
    view = new EditorView({ state, parent: host });

    // Seed the bound value from the editor's real document once at mount. In
    // read-only mode this equals the input exactly; in editable mode it is the
    // starting point the update listener then keeps in sync.
    bytes = latin1ToBytes(view.state.doc.toString());

    return () => {
      view?.destroy();
      view = undefined;
    };
  });
</script>

<div class="byte-editor" data-testid="byte-editor">
  {#if chips.length > 0}
    <div class="chip-bar" data-testid="assisted-chips">
      {#each chips as c, i (i)}
        <span class="assist-chip" data-testid="assisted-chip"
          >{c.label}{#if c.state}: {c.state}{/if}</span
        >
      {/each}
    </div>
  {/if}
  {#if diagnostic}
    <div class="diagnostic-banner" data-testid="diagnostic-banner" role="status">{diagnostic}</div>
  {/if}
  {#if pasteChip}
    <div class="paste-chip" data-testid="paste-chip" role="status">
      <span>{pasteChip}</span>
      <button type="button" aria-label="Dismiss" onclick={() => (pasteChip = null)}>×</button>
    </div>
  {/if}
  {#if !readonly}
    <div class="raw-actions">
      <button type="button" data-testid="copy-raw" onclick={copyRaw}>Copy raw</button>
    </div>
  {/if}
  <div class="cm-host" bind:this={host}></div>
</div>

<style>
  .byte-editor {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .chip-bar {
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem;
    padding: 0.3rem 0.1rem;
  }
  .assist-chip {
    font-size: 0.75rem;
    padding: 0.1rem 0.4rem;
    border-radius: 3px;
    background: #22333a;
    color: #bfe3ef;
  }
  .diagnostic-banner {
    padding: 0.4rem 0.6rem;
    background: #4a2b00;
    color: #ffd9a0;
    font-size: 0.85rem;
  }
  .paste-chip {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.3rem 0.6rem;
    background: #2a2a3a;
    color: #cfd0ff;
    font-size: 0.8rem;
  }
  .paste-chip button {
    background: none;
    border: none;
    color: inherit;
    cursor: pointer;
    font-size: 1rem;
    line-height: 1;
  }
  .raw-actions {
    display: flex;
    gap: 0.4rem;
    padding: 0.2rem 0.1rem;
  }
  .raw-actions button {
    font-size: 0.75rem;
    padding: 0.15rem 0.5rem;
    border: 1px solid #3a3a4a;
    border-radius: 3px;
    background: #22222e;
    color: #cfd0ff;
    cursor: pointer;
  }
  .cm-host {
    min-height: 6rem;
    overflow: auto;
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  }
  /* Non-printable placeholder glyphs (rendered by CodeMirror decorations). */
  .cm-host :global(.cm-nonprintable) {
    opacity: 0.55;
    border-radius: 2px;
    padding: 0 1px;
    background: rgba(128, 128, 160, 0.25);
  }
</style>
