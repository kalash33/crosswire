<script lang="ts">
  // Actions for the focused/opened History row, shown above the detail pane.
  // Every action is a visible button; shortcuts are shown next to the label.
  import Button from '../ui/Button.svelte';
  import Badge from '../ui/Badge.svelte';
  import CopyDropdown from './CopyDropdown.svelte';
  import { chordLabel } from '../copy-actions.svelte';
  import type { RowActions } from './row-actions';
  import type { Row } from '../stores/history.svelte';

  let {
    id,
    row,
    actions,
    busy = false,
    showTarget = false,
  }: {
    id: string;
    row: Row | null;
    actions: RowActions;
    busy?: boolean;
    /** Name the target row (when it is not the one open in the detail). */
    showTarget?: boolean;
  } = $props();
</script>

<div class="selbar" role="toolbar" aria-label="Exchange actions" data-testid="selection-toolbar">
  <Button
    size="sm"
    variant="primary"
    icon="repeater"
    kbd={chordLabel('send-to-repeater')}
    data-testid="send-to-repeater"
    disabled={busy}
    onclick={actions.send}>Send to Repeater</Button
  >
  <CopyDropdown {id} row={row ? { host: row.host, path: row.path } : null} testid="selection-copy" />
  <Button size="sm" icon="tag" data-testid="row-tag" onclick={actions.tag}>Tag</Button>
  <Button size="sm" icon="scope" data-testid="add-host-to-scope" disabled={!row?.host} onclick={actions.addHost}
    >Add host to scope</Button
  >
  <Button size="sm" icon="external" data-testid="selection-preview" onclick={actions.preview}>Preview</Button>
  {#if row && showTarget}
    <span class="target" title={`${row.method ?? ''} ${row.host ?? ''}${row.path ?? ''}`}>
      <Badge kind="method" value={row.method ?? '—'} />
      <span class="tpath">{row.host ?? ''}{row.path ?? ''}</span>
    </span>
  {/if}
</div>

<style>
  .selbar {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--s-1) var(--s-2);
    padding: var(--s-2) var(--s-3);
    border-bottom: 1px solid var(--border);
    background: var(--bg-elev);
    min-width: 0;
  }
  .target {
    display: inline-flex;
    align-items: center;
    gap: var(--s-1);
    margin-left: auto;
    min-width: 0;
    max-width: 40%;
    color: var(--fg-faint);
    font-size: var(--fs-xs);
  }
  .tpath {
    font-family: var(--font-mono);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }
</style>
