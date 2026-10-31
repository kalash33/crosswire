<script lang="ts">
  // A visible shortcut hint, e.g. <Kbd keys="Ctrl+R" />.
  let { keys }: { keys: string } = $props();
</script>

<kbd class="kbd">{keys}</kbd>

<style>
  .kbd {
    display: inline-flex;
    align-items: center;
    height: 18px;
    padding: 0 5px;
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    line-height: 1;
    color: var(--fg-muted);
    background: var(--bg-sunken);
    border: 1px solid var(--border);
    border-bottom-width: 2px;
    border-radius: var(--r-1);
    white-space: nowrap;
  }
</style>
