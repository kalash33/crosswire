<script lang="ts">
  import { onMount } from 'svelte';
  import { EditorState } from '@codemirror/state';
  import { EditorView } from '@codemirror/view';
  import { bytesToLatin1, latin1ToBytes, encodePastedText } from './encoding';
  import { nonPrintablePlaceholders } from './nonprintable';
  import { readCspNonce } from './csp';

  // `bytes` is bindable: in read-only use (Task 6) it never changes after mount;
  // the editable direction (Plan 5b) will push edits back through the same prop.
  // `readonly` is true in this task. `diagnostic` surfaces the partial/timed-out
  // banner (§7.2); the consumer (Task 8) supplies the text.
  let {
    bytes = $bindable(new Uint8Array()),
    readonly = true,
    diagnostic = null,
  }: {
    bytes?: Uint8Array;
    readonly?: boolean;
    diagnostic?: string | null;
  } = $props();

  let host: HTMLDivElement;
  let view: EditorView | undefined;
  let pasteChip = $state<string | null>(null);

  // Paste handling (dormant while read-only; wired for Plan 5b). Non-Latin-1
  // text cannot be mapped 1:1, so it is UTF-8 encoded and announced as a chip.
  function onPaste(ev: ClipboardEvent): void {
    if (readonly) return;
    const text = ev.clipboardData?.getData('text') ?? '';
    const { nonLatin1 } = encodePastedText(text);
    if (nonLatin1) {
      pasteChip = 'Pasted non-Latin-1 text was UTF-8 encoded.';
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
      // doc.toString() is an exact inverse of the input string.
      EditorState.lineSeparator.of('\n'),
      EditorState.readOnly.of(readonly),
      EditorView.editable.of(!readonly),
      EditorView.lineWrapping,
      nonPrintablePlaceholders,
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

    // Reflect the editor's ACTUAL document back into the bound value. Reading
    // through doc.toString() -> latin1ToBytes proves the round-trip travels the
    // real CodeMirror document; in read-only mode this equals the input exactly.
    bytes = latin1ToBytes(view.state.doc.toString());

    return () => {
      view?.destroy();
      view = undefined;
    };
  });
</script>

<div class="byte-editor" data-testid="byte-editor">
  {#if diagnostic}
    <div class="diagnostic-banner" data-testid="diagnostic-banner" role="status">{diagnostic}</div>
  {/if}
  {#if pasteChip}
    <div class="paste-chip" data-testid="paste-chip" role="status">
      <span>{pasteChip}</span>
      <button type="button" aria-label="Dismiss" onclick={() => (pasteChip = null)}>×</button>
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
