<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { HTMLSelectAttributes } from 'svelte/elements';

  type Props = Omit<HTMLSelectAttributes, 'class' | 'size'> & {
    value?: unknown;
    size?: 'sm' | 'md';
    class?: string;
    children?: Snippet;
  };

  let { value = $bindable(), size = 'md', class: klass = '', children, ...rest }: Props = $props();
</script>

<span class="select-wrap {size} {klass}">
  <select {...rest} bind:value class="select">
    {@render children?.()}
  </select>
  <svg class="caret" viewBox="0 0 16 16" aria-hidden="true" focusable="false">
    <path d="M4 6l4 4 4-4" />
  </svg>
</span>

<style>
  .select-wrap {
    position: relative;
    display: inline-flex;
    align-items: center;
    min-width: 0;
  }
  .select {
    appearance: none;
    height: var(--control-h-md);
    min-height: 0;
    width: 100%;
    padding: 0 26px 0 var(--s-2);
    font: inherit;
    font-size: var(--fs-md);
    color: var(--fg);
    background: var(--bg-sunken);
    border: 1px solid var(--border);
    border-radius: var(--r-1);
    cursor: pointer;
  }
  .sm .select {
    height: var(--control-h-sm);
    font-size: var(--fs-sm);
  }
  .select:hover {
    border-color: var(--border-strong);
  }
  .select:focus-visible {
    outline: 2px solid var(--focus);
    outline-offset: -1px;
  }
  .caret {
    position: absolute;
    right: 7px;
    width: 12px;
    height: 12px;
    fill: none;
    stroke: var(--fg-muted);
    stroke-width: 1.5;
    stroke-linecap: round;
    stroke-linejoin: round;
    pointer-events: none;
  }
</style>
