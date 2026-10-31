import { describe, expect, it } from 'vitest';
import { HEX_PAGE, hexPage } from './hex-window';

describe('hexPage', () => {
  it('pages are 32 KiB', () => {
    expect(HEX_PAGE).toBe(32768);
  });
  it('empty input is one empty page', () => {
    expect(hexPage(0, 0)).toEqual({ page: 0, pages: 1, start: 0, end: 0 });
  });
  it('a 2 MB buffer is 64 bounded pages', () => {
    const total = 2 * 1024 * 1024;
    expect(hexPage(total, 0)).toEqual({ page: 0, pages: 64, start: 0, end: 32768 });
    expect(hexPage(total, 63)).toEqual({ page: 63, pages: 64, start: 63 * 32768, end: total });
  });
  it('clamps out-of-range pages and keeps the last partial page short', () => {
    const total = 32768 + 10;
    expect(hexPage(total, 99)).toEqual({ page: 1, pages: 2, start: 32768, end: total });
    expect(hexPage(total, -3)).toEqual({ page: 0, pages: 2, start: 0, end: 32768 });
    expect(hexPage(total, Number.NaN).page).toBe(0);
    expect(hexPage(32768, 1)).toEqual({ page: 0, pages: 1, start: 0, end: 32768 });
  });
  it('every page is at most HEX_PAGE bytes', () => {
    const total = 1_000_003;
    const { pages } = hexPage(total, 0);
    for (let p = 0; p < pages; p++) {
      const w = hexPage(total, p);
      expect(w.end - w.start).toBeLessThanOrEqual(HEX_PAGE);
      expect(w.end - w.start).toBeGreaterThan(0);
    }
  });
});
