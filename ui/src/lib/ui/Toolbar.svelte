<script lang="ts">
  import type { Snippet } from 'svelte';

  let {
    label,
    dense = false,
    children,
  }: { label: string; dense?: boolean; children?: Snippet } = $props();
</script>

<div class="toolbar" class:dense role="toolbar" aria-label={label}>
  {@render children?.()}
</div>

<style>
  .toolbar {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--s-2);
    min-height: 36px;
    padding: var(--s-1) 0;
    min-width: 0;
  }
  .toolbar.dense {
    gap: var(--s-1);
    min-height: 30px;
  }
  /* A flex spacer child: <span class="spacer"></span>. */
  .toolbar :global(.spacer) {
    flex: 1 1 auto;
  }
  .toolbar :global(.sep) {
    width: 1px;
    align-self: stretch;
    margin: var(--s-1) 2px;
    background: var(--border);
  }
</style>
