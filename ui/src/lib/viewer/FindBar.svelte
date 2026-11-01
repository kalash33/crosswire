<script lang="ts">
  // Find-in-message bar (U2 D1): the query is typed text and the count is a
  // number; nothing here renders captured data. Enter / Shift+Enter step
  // through matches, Esc closes.
  import { onMount } from 'svelte';
  import IconButton from '../ui/IconButton.svelte';

  let {
    query = $bindable(''),
    caseSensitive = $bindable(false),
    count,
    capped = false,
    current,
    onnext,
    onprev,
    onclose,
    testid,
  }: {
    query?: string;
    caseSensitive?: boolean;
    count: number;
    /** More matches exist than were counted (FIND_LIMIT). */
    capped?: boolean;
    current: number;
    onnext: () => void;
    onprev: () => void;
    onclose: () => void;
    /** Base test id: `${testid}-input`, `-count`, `-next`, `-prev`, `-case`, `-close`. */
    testid: string;
  } = $props();

  let input = $state<HTMLInputElement>();
  onMount(() => {
    input?.focus();
    input?.select();
  });

  /** Focus the field again (Ctrl+F while the bar is already open). */
  export function focus(): void {
    input?.focus();
    input?.select();
  }

  function onkeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter') {
      e.preventDefault();
      if (e.shiftKey) onprev();
      else onnext();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      onclose();
    }
  }

  const status = $derived(
    query === '' ? '' : count === 0 ? 'No matches' : `${current + 1}/${count}${capped ? '+' : ''}`,
  );
</script>

<div class="findbar" role="search" data-testid={testid}>
  <input
    bind:this={input}
    bind:value={query}
    class="field"
    type="text"
    spellcheck="false"
    autocomplete="off"
    placeholder="Find in message"
    aria-label="Find in message"
    data-testid={`${testid}-input`}
    {onkeydown}
  />
  <span class="count" class:none={query !== '' && count === 0} aria-live="polite" data-testid={`${testid}-count`}
    >{status}</span
  >
  <button
    type="button"
    class="case"
    class:on={caseSensitive}
    aria-pressed={caseSensitive}
    title="Match case"
    data-testid={`${testid}-case`}
    onclick={() => (caseSensitive = !caseSensitive)}>Aa</button
  >
  <IconButton
    size="sm"
    icon="chevron-up"
    label="Previous match"
    kbd="Shift+Enter"
    disabled={count === 0}
    data-testid={`${testid}-prev`}
    onclick={onprev}
  />
  <IconButton
    size="sm"
    icon="chevron-down"
    label="Next match"
    kbd="Enter"
    disabled={count === 0}
    data-testid={`${testid}-next`}
    onclick={onnext}
  />
  <IconButton size="sm" icon="close" label="Close find" kbd="Esc" data-testid={`${testid}-close`} onclick={onclose} />
</div>

<style>
  .findbar {
    display: flex;
    align-items: center;
    gap: var(--s-1);
    padding: var(--s-1) var(--s-1) var(--s-1) var(--s-2);
    background: var(--bg-elev);
    border: 1px solid var(--border);
    border-left: 2px solid var(--accent);
    border-radius: var(--r-1);
    min-width: 0;
  }
  .field {
    flex: 1 1 12rem;
    min-width: 6rem;
    height: var(--control-h-sm);
    min-height: 0;
    padding: 0 var(--s-2);
    font-family: var(--font-mono);
    font-size: var(--fs-sm);
    color: var(--fg);
    background: var(--bg-sunken);
    border: 1px solid var(--border);
    border-radius: var(--r-1);
  }
  .count {
    min-width: 5.5em;
    font-size: var(--fs-sm);
    font-variant-numeric: tabular-nums;
    color: var(--fg-muted);
    text-align: right;
  }
  .count.none {
    color: var(--warn);
  }
  .case {
    height: var(--control-h-sm);
    min-height: 0;
    padding: 0 6px;
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--fg-muted);
    background: transparent;
    border: 1px solid transparent;
    border-radius: var(--r-1);
    cursor: pointer;
  }
  .case:hover {
    color: var(--fg);
    background: var(--bg-hover);
  }
  .case.on {
    color: var(--accent);
    background: var(--accent-soft);
  }
</style>
