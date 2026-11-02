// Pure window math for the virtualised History list. No Svelte, no DOM.
//
// Geometry: the scroll container holds a sticky header (`headH`) above the
// rows and a sticky footer (`footH`) below; row `i` occupies content y
// `headH + i * rowH`. Everything here is index arithmetic.

/** Rows rendered above and below the viewport. */
export const OVERSCAN = 8;
/** Rows before the loaded end that trigger the next page. */
export const AUTOLOAD_MARGIN = 200;

/** A render window `[start, end)` plus the spacer heights in pixels. */
export interface Win {
  start: number;
  end: number;
  before: number;
  after: number;
}

export function windowFor(
  scrollTop: number,
  viewportH: number,
  rowH: number,
  headH: number,
  total: number,
  overscan: number = OVERSCAN,
): Win {
  const first = Math.floor(Math.max(0, scrollTop - headH) / rowH);
  const visible = Math.ceil(viewportH / rowH);
  const end = Math.max(0, Math.min(total, first + visible + overscan));
  // Clamped to `end` so a data-set shrink never leaves an empty window.
  const start = Math.min(Math.max(0, first - overscan), end);
  return { start, end, before: start * rowH, after: (total - end) * rowH };
}

/** The `scrollTop` that reveals row `index`, or null if it is fully visible. */
export function revealTop(
  index: number,
  scrollTop: number,
  viewportH: number,
  rowH: number,
  headH: number,
  footH: number,
): number | null {
  const y = headH + index * rowH;
  if (y < scrollTop + headH) return Math.max(0, y - headH);
  if (y + rowH > scrollTop + viewportH - footH) return Math.max(0, y + rowH - viewportH + footH);
  return null;
}

/** The first fully visible row id and its pixel offset within the viewport. */
export function anchorAt(
  scrollTop: number,
  rowH: number,
  headH: number,
  ids: readonly string[],
): { id: string; offset: number } | null {
  const i = Math.ceil(Math.max(0, scrollTop - headH) / rowH);
  if (i >= ids.length) return null;
  return { id: ids[i], offset: headH + i * rowH - scrollTop };
}

/** The `scrollTop` that puts the anchored row back at its offset, or null. */
export function restoreAnchor(
  anchor: { id: string; offset: number },
  indexOf: (id: string) => number,
  rowH: number,
  headH: number,
): number | null {
  const i = indexOf(anchor.id);
  if (i < 0) return null;
  return headH + i * rowH - anchor.offset;
}

export function shouldAutoLoad(
  win: Win,
  total: number,
  s: { hasMore: boolean; loading: boolean; searching: boolean; capped: boolean },
): boolean {
  if (!s.hasMore || s.loading || s.searching || s.capped) return false;
  return win.end >= total - AUTOLOAD_MARGIN;
}
