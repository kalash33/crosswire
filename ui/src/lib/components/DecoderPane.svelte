<script lang="ts">
  // Chained decoder pane (Burp Decoder neighbourhood). Every value shown here
  // — outputs, warnings, JWT parts, chips — is Svelte `{...}` text interpolation
  // or a bound textarea `value`; never {@html} (§11.1). Pure client side: no
  // network, no eval-class API.
  import { onMount } from 'svelte';
  import { apply, CODECS, jwtView, type Codec, type Direction } from '../decode/codecs';
  import { bytesToLatin1 } from '../editor/encoding';
  import { fromTextareaText, toTextareaText } from '../decode/textarea';
  import HexPane from '../editor/HexPane.svelte';
  import Button from '../ui/Button.svelte';
  import IconButton from '../ui/IconButton.svelte';
  import Select from '../ui/Select.svelte';
  import Toolbar from '../ui/Toolbar.svelte';

  let {
    initial,
    initialNonLatin1 = false,
    onClose,
    embedded = false,
  }: {
    initial: Uint8Array;
    initialNonLatin1?: boolean;
    onClose: () => void;
    /** Fill the host (the /decoder route) instead of floating as a dialog. */
    embedded?: boolean;
  } = $props();

  interface Step {
    codec: Codec;
    dir: Direction;
  }

  // The textarea always shows bytesToLatin1(input) (1:1 Latin-1, §7.2), so
  // every seeded byte round-trips. Input outside Latin-1 is UTF-8 encoded and
  // flagged with a chip, exactly like the editor's paste path.
  // svelte-ignore state_referenced_locally
  let input = $state<Uint8Array>(initial);
  const text = $derived(toTextareaText(input));
  // svelte-ignore state_referenced_locally
  let nonLatin1 = $state(initialNonLatin1);
  let notice = $state<string | null>(null);
  let root: HTMLDivElement;
  let steps = $state<Step[]>([]);
  let hexOpen = $state<Record<number, boolean>>({});

  const outputs = $derived.by(() => {
    const out: { bytes: Uint8Array; warning: string | null }[] = [];
    let cur = input;
    for (const s of steps) {
      const r = apply(s.codec, s.dir, cur);
      out.push(r);
      cur = r.bytes;
    }
    return out;
  });
  const jwt = $derived(jwtView(input));

  function onInput(e: Event) {
    const el = e.currentTarget as HTMLTextAreaElement;
    const r = fromTextareaText(el.value);
    input = r.bytes;
    if (r.nonLatin1) el.value = toTextareaText(input);
    nonLatin1 = r.nonLatin1;
  }

  function addStep() {
    steps.push({ codec: 'base64', dir: 'decode' });
  }

  function removeStep(i: number) {
    steps.splice(i, 1);
    hexOpen = {};
  }

  async function copy(i: number): Promise<void> {
    try {
      await navigator.clipboard?.writeText(bytesToLatin1(outputs[i].bytes));
      notice = null;
    } catch {
      notice = 'Copy failed: clipboard unavailable.';
    }
  }

  // The pane's own dismiss; app chords stay in the keymap. Embedded in the
  // /decoder route, Escape typed inside a field (input, textarea, select,
  // CodeMirror) is left to the field so a stray Escape never discards the
  // chain; Escape anywhere else in the pane still goes back.
  function onKeydown(e: KeyboardEvent) {
    if (e.key !== 'Escape') return;
    if (embedded && e.target instanceof Element && e.target.closest(FIELD)) return;
    e.stopPropagation();
    onClose();
  }
  const FIELD = 'input, textarea, select, [contenteditable]:not([contenteditable="false"])';

  // Take focus so Escape works right after the chord; give it back on close.
  onMount(() => {
    const prev = document.activeElement;
    root.focus();
    return () => {
      if (prev instanceof HTMLElement) prev.focus();
    };
  });
</script>

<div
  class="decoder"
  class:embedded
  data-testid="decoder-pane"
  role={embedded ? 'region' : 'dialog'}
  aria-label="Decoder"
  tabindex="-1"
  bind:this={root}
  onkeydown={onKeydown}
>
  <Toolbar label="Decoder">
    {#if embedded}
      <Button size="sm" variant="ghost" icon="chevron-right" class="back" kbd="Esc" data-testid="decoder-close" onclick={onClose}
        >Back</Button
      >
    {:else}
      <strong class="title">Decoder</strong>
    {/if}
    <span class="spacer"></span>
    <span class="hint">Chain codecs; each step reads the previous output.</span>
    {#if !embedded}
      <IconButton icon="close" size="sm" label="Close" kbd="Esc" data-testid="decoder-close" onclick={onClose} />
    {/if}
  </Toolbar>

  <section class="block" aria-labelledby="decoder-input-label">
    <div class="lbl" id="decoder-input-label">Input</div>
    <textarea
      class="mono input"
      data-testid="decoder-input"
      aria-labelledby="decoder-input-label"
      rows="5"
      spellcheck="false"
      value={text}
      oninput={onInput}
    ></textarea>
    {#if nonLatin1}
      <div class="note" data-testid="decoder-input-chip">non-Latin-1 text re-encoded as UTF-8</div>
    {/if}
    {#if notice}
      <div class="note error" data-testid="decoder-copy-error">{notice}</div>
    {/if}
  </section>

  {#if jwt}
    <section class="block jwt" data-testid="decoder-jwt" aria-label="JWT">
      <div class="banner" data-testid="decoder-jwt-banner">{jwt.warnings[0]}</div>
      {#each jwt.warnings.slice(1) as w (w)}
        <div class="note warn">{w}</div>
      {/each}
      <div class="lbl">Header</div>
      <pre class="mono out">{jwt.header}</pre>
      <div class="lbl">Payload</div>
      <pre class="mono out">{jwt.payload}</pre>
      <div class="lbl">Signature</div>
      <pre class="mono out">{jwt.signature}</pre>
    </section>
  {/if}

  {#each steps as step, i (i)}
    <section class="block step" aria-label={`Step ${i + 1}`}>
      <div class="row">
        <span class="lbl n">Step {i + 1}</span>
        <Select
          size="sm"
          aria-label="Codec"
          data-testid={`decoder-step-${i}-codec`}
          value={step.codec}
          onchange={(e) => (step.codec = e.currentTarget.value as Codec)}
        >
          {#each CODECS as c (c)}
            <option value={c}>{c}</option>
          {/each}
        </Select>
        <Select
          size="sm"
          aria-label="Direction"
          data-testid={`decoder-step-${i}-dir`}
          value={step.dir}
          onchange={(e) => (step.dir = e.currentTarget.value as Direction)}
        >
          <option value="encode">encode</option>
          <option value="decode">decode</option>
        </Select>
        <span class="spacer"></span>
        <Button size="sm" icon="copy" data-testid={`decoder-copy-${i}`} onclick={() => copy(i)}>Copy</Button>
        <Button
          size="sm"
          icon="hex"
          aria-pressed={!!hexOpen[i]}
          data-testid={`decoder-hex-toggle-${i}`}
          onclick={() => (hexOpen[i] = !hexOpen[i])}>Hex</Button
        >
        <IconButton
          icon="close"
          size="sm"
          label={`Remove step ${i + 1}`}
          data-testid={`decoder-step-${i}-remove`}
          onclick={() => removeStep(i)}
        />
      </div>
      <div class="lbl" id={`decoder-out-label-${i}`}>Output</div>
      <pre class="mono out" aria-labelledby={`decoder-out-label-${i}`} data-testid={`decoder-output-${i}`}>{bytesToLatin1(outputs[i].bytes)}</pre>
      {#if outputs[i].warning}
        <div class="note warn" data-testid={`decoder-warning-${i}`}>{outputs[i].warning}</div>
      {/if}
      {#if hexOpen[i]}
        <div class="hex" data-testid={`decoder-hex-${i}`}><HexPane bytes={outputs[i].bytes} /></div>
      {/if}
    </section>
  {/each}

  <div class="add">
    <Button size="sm" icon="plus" data-testid="decoder-step-add" onclick={addStep}>Add step</Button>
  </div>
</div>

<style>
  .decoder {
    display: flex;
    flex-direction: column;
    gap: var(--s-3);
    position: fixed;
    right: var(--s-3);
    bottom: calc(var(--statusbar-h) + var(--s-2));
    z-index: var(--z-overlay);
    width: min(36rem, 92vw);
    max-height: 70vh;
    overflow: auto;
    padding: var(--s-2) var(--s-3);
    font-size: var(--fs-sm);
    background: var(--bg-overlay);
    border: 1px solid var(--border-strong);
    border-radius: var(--r-2);
    box-shadow: var(--shadow-pop);
  }
  /* Route-hosted: fill the main area, no floating chrome. */
  .decoder.embedded {
    position: static;
    width: auto;
    max-width: 64rem;
    max-height: none;
    overflow: visible;
    padding: 0;
    font-size: var(--fs-md);
    background: transparent;
    border: none;
    box-shadow: none;
  }
  .decoder:focus {
    outline: none;
  }
  .decoder :global(.back .icon) {
    transform: rotate(180deg);
  }
  .title {
    font-weight: 600;
  }
  .hint {
    font-size: var(--fs-sm);
    color: var(--fg-muted);
  }
  .block {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
    padding: var(--s-3);
    background: var(--bg-elev);
    border: 1px solid var(--border);
    border-radius: var(--r-2);
  }
  .lbl {
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--fg-muted);
  }
  .lbl.n {
    min-width: 3.5rem;
  }
  .mono {
    font-family: var(--font-mono);
  }
  .input {
    width: 100%;
    min-height: 6rem;
    box-sizing: border-box;
    resize: vertical;
    padding: var(--s-2);
    font-size: var(--fs-sm);
    color: var(--fg);
    background: var(--bg-sunken);
    border: 1px solid var(--border);
    border-radius: var(--r-1);
  }
  .out {
    margin: 0;
    min-height: 1.5em;
    max-height: 24rem;
    overflow: auto;
    padding: var(--s-2);
    font-size: var(--fs-sm);
    white-space: pre-wrap;
    word-break: break-all;
    color: var(--fg);
    background: var(--bg-sunken);
    border: 1px solid var(--border-faint);
    border-radius: var(--r-1);
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-2);
  }
  .spacer {
    flex: 1 1 auto;
  }
  .note {
    font-size: var(--fs-sm);
    color: var(--fg-muted);
  }
  .note.warn {
    color: var(--warn);
  }
  .note.error {
    color: var(--danger);
  }
  .banner {
    padding: var(--s-1) var(--s-2);
    color: var(--warn-fg);
    background: var(--warn-bg);
    border-radius: var(--r-1);
  }
  .add {
    display: flex;
  }
</style>
