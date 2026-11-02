import { describe, it, expect } from 'vitest';
import {
  OVERSCAN,
  AUTOLOAD_MARGIN,
  windowFor,
  revealTop,
  anchorAt,
  restoreAnchor,
  shouldAutoLoad,
  type Win,
} from './virtual-window';

const ROW = 26;
const HEAD = 30;
const FOOT = 40;
const VIEW = 520; // 20 rows

describe('windowFor', () => {
  it('starts at 0 at the top', () => {
    const w = windowFor(0, VIEW, ROW, HEAD, 1000);
    expect(w.start).toBe(0);
    expect(w.before).toBe(0);
    expect(w.end).toBe(Math.ceil(VIEW / ROW) + OVERSCAN);
  });

  it('windows the middle with overscan on both sides', () => {
    const w = windowFor(HEAD + 100 * ROW, VIEW, ROW, HEAD, 1000);
    expect(w.start).toBe(100 - OVERSCAN);
    expect(w.end).toBe(100 + Math.ceil(VIEW / ROW) + OVERSCAN);
    expect(w.before).toBe(w.start * ROW);
    expect(w.after).toBe((1000 - w.end) * ROW);
  });

  it('reaches total at the bottom', () => {
    const w = windowFor(HEAD + 1000 * ROW, VIEW, ROW, HEAD, 1000);
    expect(w.end).toBe(1000);
    expect(w.after).toBe(0);
  });

  it('renders everything when total is smaller than the viewport', () => {
    const w = windowFor(0, VIEW, ROW, HEAD, 5);
    expect(w).toEqual({ start: 0, end: 5, before: 0, after: 0 });
  });

  it('clamps when total shrinks below the old start', () => {
    const w = windowFor(HEAD + 500 * ROW, VIEW, ROW, HEAD, 10);
    expect(w.start).toBeLessThanOrEqual(10);
    expect(w.end).toBe(10);
    expect(w.after).toBeGreaterThanOrEqual(0);
    expect(w.before + (w.end - w.start) * ROW + w.after).toBe(10 * ROW);
  });

  it('keeps pads plus rendered rows equal to the full height and bounds the count', () => {
    const visible = Math.ceil(VIEW / ROW);
    for (const top of [0, 13, 700, 12345, 26 * 990 + HEAD, 99999999]) {
      const w = windowFor(top, VIEW, ROW, HEAD, 1000);
      expect(w.before + (w.end - w.start) * ROW + w.after).toBe(1000 * ROW);
      expect(w.end - w.start).toBeLessThanOrEqual(visible + 2 * OVERSCAN);
      expect(w.after).toBeGreaterThanOrEqual(0);
    }
  });

  it('handles an empty list', () => {
    expect(windowFor(0, VIEW, ROW, HEAD, 0)).toEqual({ start: 0, end: 0, before: 0, after: 0 });
  });
});

describe('revealTop', () => {
  it('is null for a fully visible row', () => {
    expect(revealTop(5, 0, VIEW, ROW, HEAD, FOOT)).toBeNull();
  });

  it('top-aligns a row above the viewport (below the sticky head)', () => {
    const top = revealTop(10, 50 * ROW, VIEW, ROW, HEAD, FOOT);
    expect(top).toBe(HEAD + 10 * ROW - HEAD);
  });

  it('bottom-aligns a row below the viewport (above the sticky footer)', () => {
    const top = revealTop(100, 0, VIEW, ROW, HEAD, FOOT);
    expect(top).toBe(HEAD + 100 * ROW + ROW - VIEW + FOOT);
  });

  it('never returns a negative scrollTop', () => {
    expect(revealTop(0, 5, VIEW, ROW, HEAD, FOOT)).toBe(0);
  });
});

describe('anchorAt / restoreAnchor', () => {
  const ids = Array.from({ length: 100 }, (_, i) => `r${i}`);

  it('round-trips after ids are prepended', () => {
    const top = HEAD + 40 * ROW + 7;
    const a = anchorAt(top, ROW, HEAD, ids);
    expect(a).not.toBeNull();
    const next = ['n1', 'n2', 'n3', ...ids];
    const restored = restoreAnchor(a!, (id) => next.indexOf(id), ROW, HEAD);
    expect(restored).toBe(top + 3 * ROW);
  });

  it('anchors the first fully visible row', () => {
    const a = anchorAt(HEAD + 40 * ROW + 7, ROW, HEAD, ids);
    expect(a?.id).toBe('r41');
  });

  it('is null when the anchor id is gone or there are no rows', () => {
    expect(restoreAnchor({ id: 'zz', offset: 0 }, () => -1, ROW, HEAD)).toBeNull();
    expect(anchorAt(0, ROW, HEAD, [])).toBeNull();
  });
});

describe('shouldAutoLoad', () => {
  const total = 1000;
  const near: Win = { start: 900, end: total, before: 0, after: 0 };
  const far: Win = { start: 0, end: 30, before: 0, after: 0 };
  const ok = { hasMore: true, loading: false, searching: false, capped: false };

  it('fires near the end when idle', () => {
    expect(shouldAutoLoad(near, total, ok)).toBe(true);
    expect(shouldAutoLoad({ ...far, end: total - AUTOLOAD_MARGIN }, total, ok)).toBe(true);
  });

  it('does not fire when loading, searching, capped, exhausted or far from the end', () => {
    expect(shouldAutoLoad(near, total, { ...ok, loading: true })).toBe(false);
    expect(shouldAutoLoad(near, total, { ...ok, searching: true })).toBe(false);
    expect(shouldAutoLoad(near, total, { ...ok, capped: true })).toBe(false);
    expect(shouldAutoLoad(near, total, { ...ok, hasMore: false })).toBe(false);
    expect(shouldAutoLoad(far, total, ok)).toBe(false);
  });
});
