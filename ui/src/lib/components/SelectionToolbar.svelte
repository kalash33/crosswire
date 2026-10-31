<script lang="ts">
  // Actions for the action target (the focused row, else the opened row),
  // shown above the detail pane. One home per action:
  //  - Send to Repeater, Tag and Add host to scope always live here;
  //  - Copy and Preview live in the detail (title line / Response toolbar) for
  //    the OPEN row, so they appear here only when the target is another row;
  //  - the row context menu offers everything.
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
    index = 0,
  }: {
    id: string;
    row: Row | null;
    actions: RowActions;
    busy?: boolean;
    /** True when the target is not the row open in the detail: name it. */
    showTarget?: boolean;
    /** The target's 1-based table position (0 = unknown). */
    index?: number;
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
  <Button size="sm" icon="tag" data-testid="row-tag" onclick={actions.tag}>Tag</Button>
  <Button size="sm" icon="scope" data-testid="add-host-to-scope" disabled={!row?.host} onclick={actions.addHost}
    >Add host to scope</Button
  >
  {#if showTarget}
    <CopyDropdown {id} row={row ? { host: row.host, path: row.path } : null} testid="selection-copy" />
    <Button
      size="sm"
      icon="external"
      data-testid="selection-preview"
      disabled={row?.status === null || row?.status === undefined}
      onclick={actions.preview}>Preview</Button
    >
    <span
      class="target"
      data-testid="selection-target"
      title={row ? `${row.method ?? ''} ${row.host ?? ''}${row.path ?? ''}` : id}
    >
      <span class="on">acting on {index > 0 ? `#${index}` : 'focused row'}</span>
      {#if row?.method}<Badge kind="method" value={row.method} />{/if}
      <span class="tpath">{row?.path ?? ''}</span>
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
    min-width: 0;
    max-width: 100%;
    padding: 1px var(--s-2);
    border-radius: var(--r-pill);
    background: var(--warn-bg);
    color: var(--warn-fg);
    font-size: var(--fs-xs);
  }
  .on {
    white-space: nowrap;
    font-weight: 600;
  }
  .tpath {
    font-family: var(--font-mono);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }
</style>
