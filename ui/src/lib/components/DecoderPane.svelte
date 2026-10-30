<script lang="ts">
  // Chained decoder pane (Burp Decoder neighbourhood). Every value shown here
  // — outputs, warnings, JWT parts, chips — is Svelte `{...}` text interpolation
  // or a bound textarea `value`; never {@html} (§11.1). Pure client side: no
  // network, no eval-class API.
  import { onMount } from 'svelte';
  import { apply, CODECS, jwtView, type Codec, type Direction } from '../decode/codecs';
  import { bytesToLatin1, encodePastedText } from '../editor/encoding';
  import HexPane from '../editor/HexPane.svelte';

  let { initial, onClose }: { initial: Uint8Array; onClose: () => void } = $props();

  interface Step {
    codec: Codec;
    dir: Direction;
  }

  // `text` is what the textarea shows; `input` is the byte form every step
  // consumes. Seeded bytes map 1:1 (Latin-1); typed/pasted text is UTF-8.
  // svelte-ignore state_referenced_locally
  let text = $state(bytesToLatin1(initial));
  // svelte-ignore state_referenced_locally
  let input = $state<Uint8Array>(initial);
  let nonLatin1 = $state(false);
  let steps = $state<Step[]>([]);
  let hexOpen = $state<Record<number, boolean>>({});
  let inputEl: HTMLTextAreaElement;

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
    text = (e.currentTarget as HTMLTextAreaElement).value;
    const r = encodePastedText(text);
    input = r.bytes;
    nonLatin1 = r.nonLatin1;
  }

  function addStep() {
    steps.push({ codec: 'base64', dir: 'decode' });
  }

  function removeStep(i: number) {
    steps.splice(i, 1);
    hexOpen = {};
  }

  function copy(i: number) {
    void navigator.clipboard?.writeText(bytesToLatin1(outputs[i].bytes));
  }

  // The dialog's own dismiss; app chords stay in the keymap.
  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.stopPropagation();
      onClose();
    }
  }

  onMount(() => inputEl.focus());
</script>

<div
  class="decoder"
  data-testid="decoder-pane"
  role="dialog"
  aria-label="Decoder"
  tabindex="-1"
  onkeydown={onKeydown}
>
  <div class="head">
    <strong>Decoder</strong>
    <button type="button" aria-label="Close" data-testid="decoder-close" onclick={onClose}>×</button>
  </div>

  <textarea
    class="mono input"
    data-testid="decoder-input"
    aria-label="Decoder input"
    rows="3"
    spellcheck="false"
    bind:this={inputEl}
    value={text}
    oninput={onInput}
  ></textarea>
  {#if nonLatin1}
    <div class="chip" data-testid="decoder-input-chip">non-Latin-1 text re-encoded as UTF-8</div>
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
    right: 0.75rem;
    bottom: 2.5rem;
    z-index: 20;
    width: min(36rem, 92vw);
    max-height: 70vh;
    overflow: auto;
    background: #1c1c28;
    border: 1px solid #3a3a4a;
    border-radius: 6px;
    padding: 0.5rem 0.65rem;
    box-shadow: 0 6px 24px #0008;
    font-size: 0.8rem;
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
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
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
    background: #5a3a00;
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
    border-top: 1px solid #3a3a4a;
    margin-top: 0.4rem;
    padding-top: 0.35rem;
  }
  .row {
    display: flex;
    gap: 0.35rem;
    align-items: center;
  }
</style>
