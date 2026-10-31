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

/** Context-menu items for exchange `id` (labels are fixed text, never captured data). */
export function rowMenuItems(id: string, row: UrlSource | null, a: RowActions): MenuItem[] {
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
    { label: 'Preview response', icon: 'external', testid: 'row-menu-preview', onselect: a.preview },
  ];
}
