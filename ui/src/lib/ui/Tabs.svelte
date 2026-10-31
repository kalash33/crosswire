<script lang="ts">
  type Tab = { id: string; label: string; badge?: string | number; testid?: string };

  let {
    tabs,
    active,
    onselect,
    label,
  }: {
    tabs: readonly Tab[];
    active: string;
    onselect: (id: string) => void;
    /** Accessible name of the tab list. */
    label?: string;
  } = $props();

  let list = $state<HTMLElement>();

  // Roving focus: arrow keys move between tabs and select (automatic activation).
  function onkeydown(e: KeyboardEvent) {
    const i = tabs.findIndex((t) => t.id === active);
    let next = -1;
    if (e.key === 'ArrowRight') next = (i + 1) % tabs.length;
    else if (e.key === 'ArrowLeft') next = (i - 1 + tabs.length) % tabs.length;
    else if (e.key === 'Home') next = 0;
    else if (e.key === 'End') next = tabs.length - 1;
    if (next < 0 || tabs.length === 0) return;
    e.preventDefault();
    onselect(tabs[next].id);
    list?.querySelectorAll<HTMLElement>('[role="tab"]')[next]?.focus();
  }
</script>

<div class="tabs" role="tablist" aria-label={label} bind:this={list} tabindex="-1" {onkeydown}>
  {#each tabs as t (t.id)}
    <button
      type="button"
      role="tab"
      class="tab"
      aria-selected={t.id === active}
      tabindex={t.id === active ? 0 : -1}
      data-testid={t.testid}
      onclick={() => onselect(t.id)}
    >
      <span>{t.label}</span>
      {#if t.badge !== undefined && t.badge !== ''}<span class="count">{t.badge}</span>{/if}
    </button>
  {/each}
</div>

<style>
  .tabs {
    display: flex;
    align-items: stretch;
    gap: 2px;
    border-bottom: 1px solid var(--border);
    min-width: 0;
    overflow-x: auto;
    scrollbar-width: none;
  }
  .tabs:focus {
    outline: none;
  }
  .tab {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 30px;
    min-height: 0;
    padding: 0 var(--s-3);
    font-size: var(--fs-md);
    color: var(--fg-muted);
    background: transparent;
    border: none;
    border-bottom: 2px solid transparent;
    border-radius: 0;
    margin-bottom: -1px;
    white-space: nowrap;
    cursor: pointer;
  }
  .tab:hover:not([aria-selected='true']) {
    color: var(--fg);
    background: transparent;
  }
  .tab[aria-selected='true'] {
    color: var(--fg);
    border-bottom-color: var(--accent);
  }
  .tab:focus-visible {
    outline-offset: -2px;
  }
  .count {
    min-width: 18px;
    padding: 0 5px;
    font-size: var(--fs-xs);
    line-height: 16px;
    text-align: center;
    font-variant-numeric: tabular-nums;
    color: var(--fg-muted);
    background: var(--chip-bg);
    border-radius: var(--r-pill);
  }
</style>
