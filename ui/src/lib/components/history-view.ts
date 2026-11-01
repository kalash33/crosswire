// Pure helpers behind the History view (U2 D3): keyboard focus math, the
// column picker's persisted state and the first-run split. Kept out of the
// component so they are unit-testable without layout.

/** First-run list/detail split (share given to the list); a persisted ratio wins. */
export const HISTORY_SPLIT_DEFAULT = 0.45;

/**
 * Index of the row to focus after moving `delta` rows from `current` in a list
 * of `len` rows. Clamped (no wrap); -1 when the list is empty. With no current
 * row (`current < 0`) a forward move lands on the first row and a backward one
 * on the last.
 */
export function nextIndex(current: number, delta: number, len: number): number {
  if (len <= 0) return -1;
  if (current < 0 || current >= len) return delta < 0 ? len - 1 : 0;
  return Math.min(len - 1, Math.max(0, current + delta));
}

/** Columns the picker can toggle. Host and Path are always shown. */
export const TOGGLEABLE_COLUMNS = [
  { key: 'time', label: 'Time' },
  { key: 'method', label: 'Method' },
  { key: 'status', label: 'Status' },
  { key: 'len', label: 'Length' },
  { key: 'state', label: 'State' },
  { key: 'tags', label: 'Tags' },
] as const;

export type ColumnKey = (typeof TOGGLEABLE_COLUMNS)[number]['key'];

export const COLUMNS_KEY = 'crosswire.history.columns';

const ALL_COLUMNS: readonly ColumnKey[] = TOGGLEABLE_COLUMNS.map((c) => c.key);

/** The visible toggleable columns: every one by default or when storage is unusable. */
export function loadColumns(): ColumnKey[] {
  try {
    const raw = localStorage.getItem(COLUMNS_KEY);
    if (raw === null) return [...ALL_COLUMNS];
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [...ALL_COLUMNS];
    return ALL_COLUMNS.filter((k) => parsed.includes(k));
  } catch {
    return [...ALL_COLUMNS];
  }
}

/** Persist the visible columns; storage failures are ignored. */
export function saveColumns(visible: readonly ColumnKey[]): void {
  try {
    localStorage.setItem(COLUMNS_KEY, JSON.stringify(visible));
  } catch {
    // Not persisted; the choice still applies for this page.
  }
}

/**
 * True when keystrokes on `target` are text entry (inputs, textareas, selects,
 * contenteditable such as CodeMirror). Plain-key shortcuts (`/`) never fire there.
 */
export function isTypingTarget(target: EventTarget | null): boolean {
  if (!(target instanceof Element)) return false;
  return target.closest('input, textarea, select, [contenteditable]:not([contenteditable="false"])') !== null;
}
