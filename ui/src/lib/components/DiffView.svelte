<script lang="ts">
  import {
    collapseUnchanged,
    diffLinesDetailed,
    lineNumbers,
    splitLines,
    tooLargeToDiff,
    DIFF_MAX_LINES,
    type DiffLine,
    type DiffRow,
  } from '../diff/lines';
  import Button from '../ui/Button.svelte';

  // A line diff of two responses' Raw text (head + body, Latin-1). The text is
  // captured wire data: every line is rendered as a text node only (§11.1).
  let {
    olderText,
    newerText,
    olderLabel,
    newerLabel,
    onclose,
  }: {
    olderText: string;
    newerText: string;
    /** Short descriptions of the two sends (time and status). */
    olderLabel: string;
    newerLabel: string;
    onclose: () => void;
  } = $props();

  type Result = { tooLarge: true } | { tooLarge: false; lines: DiffLine[]; exact: boolean };

  const result = $derived.by((): Result => {
    const a = splitLines(olderText);
    const b = splitLines(newerText);
    if (tooLargeToDiff(a, b)) return { tooLarge: true };
    const d = diffLinesDetailed(a, b);
    return { tooLarge: false, lines: d.lines, exact: d.exact };
  });

  const nums = $derived(result.tooLarge ? null : lineNumbers(result.lines));
  const added = $derived(result.tooLarge ? 0 : result.lines.filter((l) => l.kind === 'add').length);
  const removed = $derived(result.tooLarge ? 0 : result.lines.filter((l) => l.kind === 'del').length);

  let hideUnchanged = $state(true);
  // Folds the user opened, by the index of their first hidden line.
  let opened = $state<number[]>([]);

  const rows = $derived.by((): DiffRow[] => {
    if (result.tooLarge) return [];
    if (!hideUnchanged) return result.lines.map((l, index) => ({ ...l, index }));
    const out: DiffRow[] = [];
    for (const r of collapseUnchanged(result.lines)) {
      if (r.kind === 'fold' && opened.includes(r.from)) {
        for (let k = r.from; k < r.from + r.count; k++) out.push({ ...result.lines[k], index: k });
      } else out.push(r);
    }
    return out;
  });

  const SIGN = { same: ' ', add: '+', del: '-' } as const;
</script>

<section class="diff" data-testid="diff-view" aria-label="Response comparison">
  <header class="head">
    <div class="sides">
      <span class="side old"><span class="mark">-</span><span data-testid="diff-older">{olderLabel}</span></span>
      <span class="side new"><span class="mark">+</span><span data-testid="diff-newer">{newerLabel}</span></span>
    </div>
    <span class="spacer"></span>
    {#if !result.tooLarge}
      <span class="summary" data-testid="diff-summary">
        <span class="n-add">+{added}</span>
        <span class="n-del">-{removed}</span>
      </span>
      <label class="opt" class:on={hideUnchanged}>
        <input type="checkbox" data-testid="diff-hide-unchanged" bind:checked={hideUnchanged} />
        <span>Hide unchanged</span>
      </label>
    {/if}
    <Button size="sm" variant="ghost" icon="close" data-testid="diff-close" onclick={onclose}>Close</Button>
  </header>

  {#if result.tooLarge}
    <p class="note" data-testid="diff-too-large">
      These responses are too large to diff (over {DIFF_MAX_LINES.toLocaleString('en-US')} lines on a side).
    </p>
  {:else}
    {#if !result.exact}
      <p class="note" data-testid="diff-approximate">
        The responses differ too much for a minimal diff; some changed regions are shown as a whole block.
      </p>
    {/if}
    {#if added === 0 && removed === 0}
      <p class="note" data-testid="diff-identical">The two responses are identical.</p>
    {/if}
    <div class="body">
      {#each rows as r (r.kind === 'fold' ? `f${r.from}` : r.index)}
        {#if r.kind === 'fold'}
          <button
            type="button"
            class="fold"
            data-testid="diff-fold"
            onclick={() => (opened = [...opened, r.from])}
          >… {r.count} unchanged lines …</button>
        {:else}
          <div class="line {r.kind}" data-testid="diff-line" data-kind={r.kind}>
            <span class="no">{nums && nums.a[r.index] ? nums.a[r.index] : ''}</span>
            <span class="no">{nums && nums.b[r.index] ? nums.b[r.index] : ''}</span>
            <span class="sign">{SIGN[r.kind]}</span>
            <span class="text" data-testid="diff-text">{r.text}</span>
          </div>
        {/if}
      {/each}
    </div>
  {/if}
</section>

<style>
  .diff {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    min-width: 0;
  }
  .head {
    flex: none;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-2) var(--s-3);
    padding: var(--s-2) var(--s-3);
    border-bottom: 1px solid var(--border);
  }
  .sides {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    font-family: var(--font-mono);
    font-size: var(--fs-sm);
  }
  .side {
    display: inline-flex;
    gap: 8px;
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .mark {
    width: 1ch;
    font-weight: 700;
  }
  .old .mark {
    color: var(--diff-del-fg);
  }
  .new .mark {
    color: var(--diff-add-fg);
  }
  .spacer {
    flex: 1;
  }
  .summary {
    display: inline-flex;
    gap: 8px;
    font-family: var(--font-mono);
    font-size: var(--fs-sm);
    font-variant-numeric: tabular-nums;
  }
  .n-add {
    color: var(--diff-add-fg);
  }
  .n-del {
    color: var(--diff-del-fg);
  }
  .opt {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 22px;
    padding: 0 8px 0 6px;
    font-size: var(--fs-sm);
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
  .note {
    margin: 0;
    padding: var(--s-2) var(--s-3);
    font-size: var(--fs-sm);
    color: var(--fg-muted);
  }
  .body {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: var(--s-1) 0;
    font-family: var(--font-mono);
    font-size: var(--fs-sm);
    line-height: 1.5;
    background: var(--bg-sunken);
  }
  .line {
    display: grid;
    grid-template-columns: 4.5ch 4.5ch 2ch max-content;
    min-width: 100%;
    width: max-content;
  }
  .line.add {
    background: var(--diff-add-bg);
  }
  .line.del {
    background: var(--diff-del-bg);
  }
  .no {
    padding-right: 1ch;
    text-align: right;
    color: var(--fg-faint);
    font-variant-numeric: tabular-nums;
    user-select: none;
  }
  .sign {
    text-align: center;
    color: var(--fg-faint);
    user-select: none;
  }
  .add .sign {
    color: var(--diff-add-fg);
  }
  .del .sign {
    color: var(--diff-del-fg);
  }
  .text {
    padding-right: var(--s-3);
    white-space: pre;
  }
  .fold {
    display: block;
    width: 100%;
    padding: 2px 0 2px 11ch;
    font: inherit;
    text-align: left;
    color: var(--fg-faint);
    background: var(--bg);
    border: none;
    border-top: 1px dashed var(--border);
    border-bottom: 1px dashed var(--border);
    cursor: pointer;
  }
  .fold:hover {
    color: var(--fg);
    background: var(--bg-hover);
  }
  .fold:focus-visible {
    outline: 2px solid var(--focus);
    outline-offset: -2px;
  }
</style>
