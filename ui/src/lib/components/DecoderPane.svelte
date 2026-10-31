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

  // The dialog's own dismiss; app chords stay in the keymap.
  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.stopPropagation();
      onClose();
    }
  }

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
  <div class="head">
    <strong>Decoder</strong>
    <button
      type="button"
      aria-label={embedded ? 'Close the decoder and go back' : 'Close'}
      title={embedded ? 'Back (Esc)' : 'Close (Esc)'}
      data-testid="decoder-close"
      onclick={onClose}>×</button
    >
  </div>

  <textarea
    class="mono input"
    data-testid="decoder-input"
    aria-label="Decoder input"
    rows="3"
    spellcheck="false"
    value={text}
    oninput={onInput}
  ></textarea>
  {#if nonLatin1}
    <div class="chip" data-testid="decoder-input-chip">non-Latin-1 text re-encoded as UTF-8</div>
  {/if}

  {#if notice}
    <div class="chip" data-testid="decoder-copy-error">{notice}</div>
  {/if}

  {#if jwt}
    <div class="jwt" data-testid="decoder-jwt">
      <div class="banner" data-testid="decoder-jwt-banner">{jwt.warnings[0]}</div>
      {#each jwt.warnings.slice(1) as w (w)}
        <div class="warn">{w}</div>
      {/each}
      <div class="lbl">Header</div>
      <pre class="mono">{jwt.header}</pre>
      <div class="lbl">Payload</div>
      <pre class="mono">{jwt.payload}</pre>
      <div class="lbl">Signature</div>
      <pre class="mono">{jwt.signature}</pre>
    </div>
  {/if}

  {#each steps as step, i (i)}
    <div class="step">
      <div class="row">
        <select
          aria-label="Codec"
          data-testid={`decoder-step-${i}-codec`}
          value={step.codec}
          onchange={(e) => (step.codec = e.currentTarget.value as Codec)}
        >
          {#each CODECS as c (c)}
            <option value={c}>{c}</option>
          {/each}
        </select>
        <select
          aria-label="Direction"
          data-testid={`decoder-step-${i}-dir`}
          value={step.dir}
          onchange={(e) => (step.dir = e.currentTarget.value as Direction)}
        >
          <option value="encode">encode</option>
          <option value="decode">decode</option>
        </select>
        <button type="button" data-testid={`decoder-copy-${i}`} onclick={() => copy(i)}>Copy</button>
        <button type="button" data-testid={`decoder-hex-toggle-${i}`} onclick={() => (hexOpen[i] = !hexOpen[i])}>
          Hex
        </button>
        <button type="button" data-testid={`decoder-step-${i}-remove`} onclick={() => removeStep(i)}>Remove</button>
      </div>
      <pre class="mono out" data-testid={`decoder-output-${i}`}>{bytesToLatin1(outputs[i].bytes)}</pre>
      {#if outputs[i].warning}
        <div class="warn" data-testid={`decoder-warning-${i}`}>{outputs[i].warning}</div>
      {/if}
      {#if hexOpen[i]}
        <div data-testid={`decoder-hex-${i}`}><HexPane bytes={outputs[i].bytes} /></div>
      {/if}
    </div>
  {/each}

  <button type="button" data-testid="decoder-step-add" onclick={addStep}>Add step</button>
</div>

<style>
  .decoder {
    position: fixed;
    right: var(--s-3);
    /* Float just above the pinned status bar row of the shell grid. */
    bottom: calc(var(--statusbar-h) + var(--s-2));
    z-index: var(--z-overlay);
    width: min(36rem, 92vw);
    max-height: 70vh;
    overflow: auto;
    background: var(--bg-overlay);
    border: 1px solid var(--border-strong);
    border-radius: 6px;
    padding: 0.5rem 0.65rem;
    box-shadow: var(--shadow-pop);
    font-size: 0.8rem;
  }
  /* Route-hosted: fill the main area, no floating chrome. */
  .decoder.embedded {
    position: static;
    width: auto;
    max-width: 64rem;
    max-height: none;
    overflow: visible;
    padding: 0;
    background: transparent;
    border: none;
    box-shadow: none;
    font-size: var(--fs-md);
  }
  .decoder.embedded:focus {
    outline: none;
  }
  .decoder.embedded .input {
    min-height: 8rem;
  }
  .head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 0.35rem;
  }
  .head button {
    background: none;
    border: none;
    color: inherit;
    cursor: pointer;
    font-size: 1rem;
    line-height: 1;
  }
  .mono {
    font-family: var(--font-mono);
  }
  .input {
    width: 100%;
    box-sizing: border-box;
    resize: vertical;
  }
  .chip,
  .warn {
    margin: 0.25rem 0;
    opacity: 0.85;
  }
  .banner {
    background: var(--warn-bg);
    color: var(--warn-fg);
    padding: 0.2rem 0.4rem;
    border-radius: 4px;
    margin: 0.35rem 0 0.2rem;
  }
  .lbl {
    opacity: 0.7;
  }
  pre {
    margin: 0.15rem 0 0.4rem;
    white-space: pre-wrap;
    word-break: break-all;
  }
  .step {
    border-top: 1px solid var(--border-strong);
    margin-top: 0.4rem;
    padding-top: 0.35rem;
  }
  .row {
    display: flex;
    gap: 0.35rem;
    align-items: center;
  }
</style>
