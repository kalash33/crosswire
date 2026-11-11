// Pure helpers behind the History view (U2 D3): keyboard focus math, the
// column picker's persisted state and the first-run split. Kept out of the
// component so they are unit-testable without layout.

/** First-run list/detail split (share given to the list); a persisted ratio wins. */
export const HISTORY_SPLIT_DEFAULT = 0.45;
/** First-run split below `HISTORY_SPLIT_WIDE_MIN` px of window: the list gets more room. */
export const HISTORY_SPLIT_NARROW = 0.55;
const HISTORY_SPLIT_WIDE_MIN = 1600;

/**
 * The first-run split for a window `width` px wide. On a laptop-width window
 * (up to 1440 px and a little beyond) the fixed History columns, sized for
 * wide UI fonts such as DejaVu Sans, leave Host and Path too little room at
 * 45%, so the list starts at 55% (the detail pane keeps well over its minimum).
 */
export function historySplitDefault(width: number): number {
  return width > 0 && width < HISTORY_SPLIT_WIDE_MIN ? HISTORY_SPLIT_NARROW : HISTORY_SPLIT_DEFAULT;
}

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

/** Rows PageUp/PageDown move: one short of a viewport (keeps context), at least one. */
export function pageStep(visible: number): number {
  return Math.max(1, Math.floor(visible) - 1);
}

/**
 * The row index a list key moves focus to from `current` in a list of `len`
 * rows with `visible` rows on screen, or null when the key is not a list
 * navigation key. `j`/`k` and the arrows move one row, PageDown/PageUp a
 * screenful, Home/End to the first/last loaded row. Clamped, no wrap; -1 for
 * an empty list.
 */
export function keyTarget(key: string, current: number, len: number, visible: number): number | null {
  switch (key) {
    case 'j':
    case 'ArrowDown':
      return nextIndex(current, 1, len);
    case 'k':
    case 'ArrowUp':
      return nextIndex(current, -1, len);
    case 'PageDown':
      return nextIndex(current, pageStep(visible), len);
    case 'PageUp':
      return nextIndex(current, -pageStep(visible), len);
    case 'Home':
      return len > 0 ? 0 : -1;
    case 'End':
      return len - 1;
    default:
      return null;
  }
}

/**
 * The Load more button's label. "End" wins over "Limit reached": a project
 * whose last page lands exactly on the row cap has nothing left to load.
 */
export function loadMoreLabel(s: { loading: boolean; hasMore: boolean; capped: boolean }): string {
  if (s.loading) return 'Loading…';
  if (!s.hasMore) return 'End';
  return s.capped ? 'Limit reached' : 'Load more';
}

/** `aria-rowcount` for the table: -1 (unknown) while pages remain, else rows + the header row. */
export function ariaRowCount(rows: number, hasMore: boolean, capped: boolean): number {
  return hasMore && !capped ? -1 : rows + 1;
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
 * contenteditable such as CodeMirror). Guards plain keys such as Esc in the
 * detail pane; keymap shortcuts like `/` apply their own typing guard.
 */
export function isTypingTarget(target: EventTarget | null): boolean {
  if (!(target instanceof Element)) return false;
  return target.closest('input, textarea, select, [contenteditable]:not([contenteditable="false"])') !== null;
}
