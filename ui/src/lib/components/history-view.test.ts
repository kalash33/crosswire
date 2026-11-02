import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import Splitter from '../ui/Splitter.svelte';
import {
  COLUMNS_KEY,
  HISTORY_SPLIT_DEFAULT,
  TOGGLEABLE_COLUMNS,
  ariaRowCount,
  isTypingTarget,
  keyTarget,
  loadColumns,
  loadMoreLabel,
  nextIndex,
  pageStep,
  saveColumns,
} from './history-view';

describe('pageStep', () => {
  it('is one row short of a viewport, never below one', () => {
    expect(pageStep(30)).toBe(29);
    expect(pageStep(2)).toBe(1);
    expect(pageStep(1)).toBe(1);
    expect(pageStep(0)).toBe(1);
  });
});

describe('keyTarget', () => {
  // [key, current, len, visible] -> target index (null: not a navigation key)
  const table: [string, number, number, number, number | null][] = [
    ['j', 3, 10, 5, 4],
    ['ArrowDown', 3, 10, 5, 4],
    ['k', 3, 10, 5, 2],
    ['ArrowUp', 3, 10, 5, 2],
    ['j', 9, 10, 5, 9],
    ['k', 0, 10, 5, 0],
    ['Home', 7, 10, 5, 0],
    ['End', 2, 10, 5, 9],
    ['Home', -1, 10, 5, 0],
    ['End', -1, 10, 5, 9],
    ['PageDown', 0, 100, 30, 29],
    ['PageUp', 50, 100, 30, 21],
    ['PageDown', 90, 100, 30, 99],
    ['PageUp', 10, 100, 30, 0],
    ['j', -1, 10, 5, 0],
    ['k', -1, 10, 5, 9],
    ['Home', 0, 0, 5, -1],
    ['End', 0, 0, 5, -1],
    ['Enter', 3, 10, 5, null],
    [' ', 3, 10, 5, null],
    ['J', 3, 10, 5, null],
    ['Tab', 3, 10, 5, null],
  ];
  for (const [key, cur, len, vis, want] of table) {
    it(`${JSON.stringify(key)} from ${cur} of ${len} (visible ${vis}) -> ${want}`, () => {
      expect(keyTarget(key, cur, len, vis)).toBe(want);
    });
  }
});

describe('loadMoreLabel', () => {
  it('shows End before Limit reached when nothing is left', () => {
    expect(loadMoreLabel({ loading: false, hasMore: true, capped: false })).toBe('Load more');
    expect(loadMoreLabel({ loading: true, hasMore: true, capped: false })).toBe('Loading…');
    expect(loadMoreLabel({ loading: false, hasMore: false, capped: false })).toBe('End');
    expect(loadMoreLabel({ loading: false, hasMore: false, capped: true })).toBe('End');
    expect(loadMoreLabel({ loading: false, hasMore: true, capped: true })).toBe('Limit reached');
  });
});

describe('ariaRowCount', () => {
  it('is -1 while the total is unknown, else rows plus the header', () => {
    expect(ariaRowCount(100, true, false)).toBe(-1);
    expect(ariaRowCount(100, false, false)).toBe(101);
    expect(ariaRowCount(50_000, true, true)).toBe(50_001);
    expect(ariaRowCount(0, false, false)).toBe(1);
  });
});

describe('nextIndex', () => {
  it('moves by delta and clamps at both ends', () => {
    expect(nextIndex(2, 1, 5)).toBe(3);
    expect(nextIndex(2, -1, 5)).toBe(1);
    expect(nextIndex(4, 1, 5)).toBe(4);
    expect(nextIndex(0, -1, 5)).toBe(0);
  });

  it('lands on the first row going down and the last going up from no focus', () => {
    expect(nextIndex(-1, 1, 5)).toBe(0);
    expect(nextIndex(-1, -1, 5)).toBe(4);
    expect(nextIndex(9, 1, 5)).toBe(0);
  });

  it('returns -1 for an empty list', () => {
    expect(nextIndex(0, 1, 0)).toBe(-1);
    expect(nextIndex(-1, -1, 0)).toBe(-1);
  });
});

describe('column persistence', () => {
  beforeEach(() => localStorage.clear());

  it('defaults to every toggleable column', () => {
    expect(loadColumns()).toEqual(TOGGLEABLE_COLUMNS.map((c) => c.key));
  });

  it('round-trips the visible set in canonical order', () => {
    saveColumns(['tags', 'time']);
    expect(localStorage.getItem(COLUMNS_KEY)).toBe('["tags","time"]');
    expect(loadColumns()).toEqual(['time', 'tags']);
  });

  it('falls back to the default on corrupt storage and ignores unknown keys', () => {
    localStorage.setItem(COLUMNS_KEY, '{not json');
    expect(loadColumns()).toHaveLength(6);
    localStorage.setItem(COLUMNS_KEY, '"x"');
    expect(loadColumns()).toHaveLength(6);
    localStorage.setItem(COLUMNS_KEY, '["host","status"]');
    expect(loadColumns()).toEqual(['status']);
  });
});

describe('first-run split', () => {
  let c: ReturnType<typeof mount> | undefined;
  beforeEach(() => localStorage.clear());
  afterEach(() => {
    if (c) unmount(c);
    c = undefined;
    document.body.replaceChildren();
  });

  function ratioPercent(): string | null {
    const target = document.createElement('div');
    document.body.append(target);
    c = mount(Splitter, {
      target,
      props: {
        storageKey: 'history-detail',
        initial: HISTORY_SPLIT_DEFAULT,
        a: ((anchor: Node) => void anchor) as never,
        b: ((anchor: Node) => void anchor) as never,
      },
    });
    flushSync();
    return target.querySelector('[role="separator"]')!.getAttribute('aria-valuenow');
  }

  it('is 45/55 with nothing persisted', () => {
    expect(HISTORY_SPLIT_DEFAULT).toBe(0.45);
    expect(ratioPercent()).toBe('45');
  });

  it('lets the persisted ratio win', () => {
    localStorage.setItem('crosswire.split.history-detail', '0.7000');
    expect(ratioPercent()).toBe('70');
  });
});

describe('isTypingTarget', () => {
  it('is true for fields and contenteditable, false for rows, buttons and non-elements', () => {
    const el = (tag: string, attrs: Record<string, string> = {}) => {
      const e = document.createElement(tag);
      for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, v);
      return e;
    };
    expect(isTypingTarget(el('input'))).toBe(true);
    expect(isTypingTarget(el('textarea'))).toBe(true);
    expect(isTypingTarget(el('select'))).toBe(true);
    const editable = el('div', { contenteditable: 'true' });
    const inner = el('b');
    editable.append(inner);
    expect(isTypingTarget(inner)).toBe(true);
    expect(isTypingTarget(el('div', { tabindex: '0' }))).toBe(false);
    expect(isTypingTarget(el('button'))).toBe(false);
    expect(isTypingTarget(null)).toBe(false);
  });
});
