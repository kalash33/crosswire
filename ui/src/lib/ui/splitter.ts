// Pure helpers behind Splitter.svelte, kept separate so they are unit-testable
// without layout.

/** Width (row) or height (column) of the drag handle, in px. Mirrors the CSS. */
export const HANDLE_PX = 5;

const PREFIX = 'crosswire.split.';

/**
 * Clamp `ratio` (share of the space given to pane A) so that pane A gets at
 * least `min[0]` px and pane B at least `min[1]` px of `total` px (handle
 * included). When both minimums cannot fit, split in proportion to them.
 * An unknown size (0) only bounds the ratio to [0, 1].
 */
export function clampRatio(ratio: number, total: number, min: readonly [number, number]): number {
  const r = Number.isFinite(ratio) ? ratio : 0.5;
  const avail = total - HANDLE_PX;
  if (!(avail > 0)) return Math.min(1, Math.max(0, r));
  const lo = min[0] / avail;
  const hi = 1 - min[1] / avail;
  if (lo > hi) return min[0] / (min[0] + min[1]);
  return Math.min(hi, Math.max(lo, r));
}

/** The persisted ratio for `key`, or null when absent, invalid or unreadable. */
export function loadRatio(key: string): number | null {
  try {
    const raw = localStorage.getItem(PREFIX + key);
    if (raw === null) return null;
    const n = Number(raw);
    return Number.isFinite(n) && n >= 0 && n <= 1 ? n : null;
  } catch {
    return null;
  }
}

/** Persist `ratio` for `key`; storage failures are ignored. */
export function saveRatio(key: string, ratio: number): void {
  try {
    localStorage.setItem(PREFIX + key, ratio.toFixed(4));
  } catch {
    // Not persisted; the split still applies for this page load.
  }
}
