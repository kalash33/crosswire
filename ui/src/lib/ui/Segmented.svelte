<script lang="ts" generics="T extends string">
  import Icon from './Icon.svelte';
  import type { IconName } from './icons';

  type Option = { value: T; label: string; icon?: IconName; testid?: string };

  let {
    options,
    value,
    onchange,
    label,
    size = 'md',
  }: {
    options: readonly Option[];
    value: T;
    onchange: (v: T) => void;
    /** Accessible name of the group. */
    label?: string;
    size?: 'sm' | 'md';
  } = $props();
</script>

<div class="segmented {size}" role="group" aria-label={label}>
  {#each options as o (o.value)}
    <button
      type="button"
      class="seg"
      class:on={o.value === value}
      aria-pressed={o.value === value}
      data-testid={o.testid}
      onclick={() => {
        if (o.value !== value) onchange(o.value);
      }}
    >
      {#if o.icon}<Icon name={o.icon} size={14} />{/if}
      <span>{o.label}</span>
    </button>
  {/each}
</div>

<style>
  .segmented {
    display: inline-flex;
    padding: 2px;
    gap: 2px;
    background: var(--bg-sunken);
    border: 1px solid var(--border);
    border-radius: var(--r-2);
  }
  .seg {
    display: inline-flex;
    align-items: center;
    gap: var(--s-1);
    height: 22px;
    min-height: 0;
    padding: 0 10px;
    font-size: var(--fs-sm);
    font-weight: 500;
    color: var(--fg-muted);
    background: transparent;
    border: none;
    border-radius: var(--r-1);
    cursor: pointer;
  }
  .sm .seg {
    height: 18px;
    padding: 0 var(--s-2);
    font-size: var(--fs-xs);
  }
  .seg:hover:not(.on) {
    color: var(--fg);
    background: var(--bg-hover);
  }
  .seg.on {
    color: var(--fg);
    background: var(--bg-active);
    box-shadow: inset 0 0 0 1px var(--border-strong);
  }
</style>
