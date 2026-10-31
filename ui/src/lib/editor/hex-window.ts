// Paging for the detail Hex tab (U1 Review Focus 2): a large body is shown one
// bounded 64 KiB window at a time, never as one DOM node per byte of the whole
// buffer.

export const HEX_PAGE = 64 * 1024;

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
