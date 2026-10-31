// Paging for the detail Hex tab (U1 Review Focus 2): a large body is shown one
// bounded 32 KiB window at a time, never as one DOM node per byte of the whole
// buffer. Halved from 64 KiB after U1 Task 5 timed a page flip on a 2 MB body
// (median of 3, Chromium): 64 KiB ~415 ms, 32 KiB ~192 ms. The cost is the
// HexPane row render (~6 ms per KiB), so a further cut belongs there.

export const HEX_PAGE = 32 * 1024;

export interface HexWindow {
  page: number;
  pages: number;
  start: number;
  end: number;
}

/** The clamped window for `page` over `total` bytes. */
export function hexPage(total: number, page: number, size: number = HEX_PAGE): HexWindow {
  const len = Math.max(0, Math.floor(total) || 0);
  const pages = Math.max(1, Math.ceil(len / size));
  const p = Math.min(pages - 1, Math.max(0, Math.floor(page) || 0));
  const start = p * size;
  return { page: p, pages, start, end: Math.min(len, start + size) };
}
