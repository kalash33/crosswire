<script lang="ts">
  // A monospace value with a copy button (Start panel, Settings). The value is
  // text-interpolated; it is never a link.
  import IconButton from '../ui/IconButton.svelte';
  import { copyWithToast } from '../clipboard';

  let {
    value,
    what,
    testid,
    wrap = false,
  }: { value: string; what: string; testid?: string; wrap?: boolean } = $props();
</script>

<div class="copy-value">
  <code class="v" class:wrap data-testid={testid}>{value}</code>
  <IconButton
    icon="copy"
    size="sm"
    label={`Copy ${what}`}
    data-testid={testid ? `${testid}-copy` : undefined}
    onclick={() => void copyWithToast(value, what)}
  />
</div>

<style>
  .copy-value {
    display: flex;
    align-items: center;
    gap: var(--s-1);
    min-width: 0;
    padding: 2px 2px 2px var(--s-2);
    background: var(--bg-sunken);
    border: 1px solid var(--border);
    border-radius: var(--r-1);
  }
  .v {
    flex: 1 1 auto;
    min-width: 0;
    overflow-x: auto;
    white-space: nowrap;
    font-family: var(--font-mono);
    font-size: var(--fs-sm);
    color: var(--fg);
    background: none;
    padding: 3px 0;
  }
  .v.wrap {
    overflow-x: visible;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
</style>
