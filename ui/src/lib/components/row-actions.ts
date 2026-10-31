// The per-exchange actions offered by the History selection toolbar and the
// row context menu (U1 spec D4): one list, so both surfaces stay in step.

import type { MenuItem } from '../ui/Menu.svelte';
import type { UrlSource } from '../copy-url';
import { chordLabel, copyItems } from '../copy-actions.svelte';

export interface RowActions {
  open: () => void;
  send: () => void;
  tag: () => void;
  addHost: () => void;
  preview: () => void;
}

/**
 * The ONE target the toolbar buttons and the Ctrl+R chord act on: the focused
 * row first (the keyboard loop), then the opened row. `differs` is true when
 * that is not the row open in the detail, so the toolbar names it (`index` is
 * its 1-based position in the table, 0 when it is not in `rows`).
 */
export function actionTarget(
  focused: string | null,
  opened: string | null,
  rows: readonly { id: string }[],
): { id: string | null; differs: boolean; index: number } {
  const id = focused ?? opened;
  if (id === null) return { id: null, differs: false, index: 0 };
  return { id, differs: id !== opened, index: rows.findIndex((r) => r.id === id) + 1 };
}

/** Context-menu items for exchange `id` (labels are fixed text, never captured data). */
export function rowMenuItems(
  id: string,
  row: (UrlSource & { status?: number | null }) | null,
  a: RowActions,
): MenuItem[] {
  return [
    { label: 'Open', testid: 'row-menu-open', onselect: a.open },
    {
      label: 'Send to Repeater',
      icon: 'repeater',
      kbd: chordLabel('send-to-repeater'),
      testid: 'row-menu-send',
      onselect: a.send,
    },
    ...copyItems(id, row),
    { label: 'Tag…', icon: 'tag', testid: 'row-menu-tag', onselect: a.tag },
    {
      label: 'Add host to scope',
      icon: 'scope',
      testid: 'row-menu-add-host',
      disabled: !row?.host,
      onselect: a.addHost,
    },
    {
      label: 'Preview response',
      icon: 'external',
      testid: 'row-menu-preview',
      disabled: row?.status === null || row?.status === undefined,
      onselect: a.preview,
    },
  ];
}
