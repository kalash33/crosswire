<script lang="ts">
  import type { Snippet } from 'svelte';
  import Icon from './Icon.svelte';

  let {
    tone = 'neutral',
    onremove,
    removeLabel = 'Remove',
    children,
  }: {
    tone?: 'neutral' | 'accent' | 'ok' | 'warn' | 'info';
    /** When set, a remove (×) button is rendered. */
    onremove?: () => void;
    /** Accessible name of the remove button. */
    removeLabel?: string;
    children?: Snippet;
  } = $props();
</script>

<span class="chip {tone}">
  <span class="text">{@render children?.()}</span>
  {#if onremove}
    <button type="button" class="rm" aria-label={removeLabel} title={removeLabel} onclick={onremove}>
      <Icon name="close" size={12} />
    </button>
  {/if}
</span>

<style>
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    max-width: 100%;
    height: 20px;
    padding: 0 7px;
    font-size: var(--fs-sm);
    line-height: 1;
    color: var(--chip-fg);
    background: var(--chip-bg);
    border-radius: var(--r-pill);
    white-space: nowrap;
  }
  .text {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .accent {
    color: var(--accent);
    background: var(--accent-soft);
  }
  .ok {
    color: var(--ok-fg);
    background: var(--ok-bg);
  }
  .warn {
    color: var(--warn-fg);
    background: var(--warn-bg);
  }
  .info {
    color: var(--info-fg);
    background: var(--info-bg);
  }
  .rm {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 16px;
    min-height: 0;
    margin-right: -4px;
    padding: 0;
    color: inherit;
    background: transparent;
    border: none;
    border-radius: var(--r-pill);
    opacity: 0.7;
    cursor: pointer;
  }
  .rm:hover {
    opacity: 1;
    background: var(--bg-hover);
  }
</style>
