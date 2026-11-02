import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import Splitter from '../ui/Splitter.svelte';
import {
  COLUMNS_KEY,
  HISTORY_SPLIT_DEFAULT,
  TOGGLEABLE_COLUMNS,
  isTypingTarget,
  loadColumns,
  nextIndex,
  saveColumns,
} from './history-view';

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
