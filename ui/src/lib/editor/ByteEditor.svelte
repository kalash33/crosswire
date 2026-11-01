<script lang="ts">
  import { onMount } from 'svelte';
  import { Compartment, EditorState, type Extension } from '@codemirror/state';
  import { Decoration, EditorView } from '@codemirror/view';
  import { bytesToLatin1, latin1ToBytes, encodePastedText } from './encoding';
  import { lineEndMarkers, nonPrintablePlaceholders } from './nonprintable';
  import { readCspNonce } from './csp';
  import { languageExtension, type EditorLanguage } from './lang';
  import type { Chip } from './chips';
  import type { EditorMatches } from '../viewer/find';
  import './highlight.css';

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
    showCopyRaw = true,
    text = null,
    language = null,
    wrap = true,
    showControls = undefined,
    matches = null,
  }: {
    bytes?: Uint8Array;
    readonly?: boolean;
    diagnostic?: string | null;
    chips?: Chip[];
    /** Show the inline "Copy raw" button (editable mode only). Off when a host toolbar provides it. */
    showCopyRaw?: boolean;
    /**
     * Read-only derived TEXT to show instead of `bytes` (the viewer's Pretty
     * JSON, which may hold characters above U+00FF). `bytes` is then ignored.
     */
    text?: string | null;
    /** Syntax colouring (class-only, see lang/tokens.ts); null = none. */
    language?: EditorLanguage | null;
    /** Soft-wrap long lines (the default, as before U2). */
    wrap?: boolean;
    /**
     * The viewer's control-character toggle. Undefined (every non-viewer
     * host) keeps the pre-U2 look: CR glyphs shown, no LF glyphs. true adds
     * `␊` glyphs at line ends; false hides the CR and LF glyphs with CSS.
     * The document bytes are never touched either way.
     */
    showControls?: boolean;
    /** Find matches to mark (the viewer's find bar). */
    matches?: EditorMatches | null;
  } = $props();

  let host: HTMLDivElement;
  let view: EditorView | undefined;
  const wrapping = new Compartment();
  const colouring = new Compartment();
  const marking = new Compartment();

  const MATCH = Decoration.mark({ class: 'cm-find-match' });
  const CURRENT = Decoration.mark({ class: 'cm-find-match cm-find-current' });

  function matchExtension(m: EditorMatches | null): Extension {
    if (!m || m.positions.length === 0 || m.length <= 0) return [];
    const len = view?.state.doc.length ?? Infinity;
    const ranges = m.positions
      .filter((p) => p + m.length <= len)
      .map((p, i) => (i === m.current ? CURRENT : MATCH).range(p, p + m.length));
    return EditorView.decorations.of(Decoration.set(ranges, true));
  }

  // Live prop changes reconfigure the mounted editor in place.
  $effect(() => {
    const w = wrap;
    view?.dispatch({ effects: wrapping.reconfigure(w ? EditorView.lineWrapping : []) });
  });
  $effect(() => {
    const l = language;
    view?.dispatch({ effects: colouring.reconfigure(languageExtension(l)) });
  });
  $effect(() => {
    const m = matches;
    if (!view) return;
    const effects = [marking.reconfigure(matchExtension(m))];
    const at = m && m.current >= 0 ? m.positions[m.current] : undefined;
    if (at !== undefined && at <= view.state.doc.length) {
      effects.push(EditorView.scrollIntoView(at, { y: 'center' }));
    }
    view.dispatch({ effects });
  });
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
      wrapping.of(wrap ? EditorView.lineWrapping : []),
      colouring.of(languageExtension(language)),
      marking.of([]),
      ...(showControls === undefined ? [] : [lineEndMarkers]),
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
      doc: text ?? bytesToLatin1(bytes),
      extensions,
    });
    view = new EditorView({ state, parent: host });
    if (matches) view.dispatch({ effects: marking.reconfigure(matchExtension(matches)) });

    // Seed the bound value from the editor's real document once at mount. In
    // read-only mode this equals the input exactly; in editable mode it is the
    // starting point the update listener then keeps in sync. A derived `text`
    // document is not bytes, so it never flows back.
    if (text === null) bytes = latin1ToBytes(view.state.doc.toString());

    return () => {
      view?.destroy();
      view = undefined;
    };
  });
</script>

<div
  class="byte-editor"
  class:hide-controls={showControls === false}
  class:show-controls={showControls === true}
  data-testid="byte-editor"
>
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
  {#if !readonly && showCopyRaw}
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
    background: var(--info-bg);
    color: var(--info-fg);
  }
  .diagnostic-banner {
    padding: 0.4rem 0.6rem;
    background: var(--warn-bg);
    color: var(--warn-fg);
    font-size: 0.85rem;
  }
  .paste-chip {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.3rem 0.6rem;
    background: var(--chip-bg);
    color: var(--fg);
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
    border: 1px solid var(--border-strong);
    border-radius: 3px;
    background: var(--bg-elev);
    color: var(--fg);
    cursor: pointer;
  }
  .cm-host {
    min-height: 6rem;
    overflow: auto;
    font-family: var(--font-mono);
  }
  /* CodeMirror chrome on theme tokens. Its base theme ships light-mode caret
     and selection colours with compound selectors, hence the specificity. */
  .cm-host :global(.cm-editor) {
    color: var(--fg);
    background: var(--bg-sunken);
  }
  .cm-host :global(.cm-editor .cm-scroller) {
    font-family: var(--font-mono);
  }
  .cm-host :global(.cm-editor.cm-focused) {
    outline: 1px solid var(--focus);
  }
  .cm-host :global(.cm-editor .cm-cursor) {
    border-left-color: var(--fg);
  }
  .cm-host :global(.cm-editor .cm-selectionLayer .cm-selectionBackground),
  .cm-host :global(.cm-editor.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground) {
    background: var(--accent-soft);
  }
  /* The viewer's control-character toggle: CR/LF glyphs collapse (bytes stay). */
  .hide-controls :global(.cm-ctl) {
    display: none;
  }
  .cm-host :global(.cm-find-match) {
    background: var(--find-match);
    border-radius: 2px;
  }
  .cm-host :global(.cm-find-current) {
    background: var(--find-current);
    outline: 1px solid var(--accent);
  }
  /* Non-printable placeholder glyphs (rendered by CodeMirror decorations). */
  .cm-host :global(.cm-nonprintable) {
    opacity: 0.55;
    border-radius: 2px;
    padding: 0 1px;
    background: var(--nonprintable-bg);
  }
</style>
