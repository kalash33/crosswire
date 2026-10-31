<script lang="ts">
  // Right-click (or the context-menu key) on a History row: the same actions
  // as the selection toolbar, plus Open.
  import Menu from '../ui/Menu.svelte';
  import { rowMenuItems, type RowActions } from './row-actions';
  import type { UrlSource } from '../copy-url';

  let {
    id,
    row,
    open,
    anchor,
    actions,
    onclose,
  }: {
    id: string;
    row: (UrlSource & { status?: number | null }) | null;
    open: boolean;
    anchor: { x: number; y: number } | HTMLElement | null;
    actions: RowActions;
    onclose: () => void;
  } = $props();

  const items = $derived(rowMenuItems(id, row, actions));
</script>

<Menu {items} {open} {anchor} {onclose} label="Exchange actions" testid="row-menu" />
