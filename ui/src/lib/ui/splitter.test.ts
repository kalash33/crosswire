import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRawSnippet, flushSync, mount, unmount } from 'svelte';
import Splitter from './Splitter.svelte';
import { HANDLE_PX, clampRatio, loadRatio, saveRatio } from './splitter';

describe('clampRatio', () => {
  it('keeps a ratio that respects both minimums', () => {
    expect(clampRatio(0.5, 1000 + HANDLE_PX, [200, 240])).toBe(0.5);
  });
  it('clamps the first pane to its minimum', () => {
    expect(clampRatio(0.05, 1000 + HANDLE_PX, [200, 240])).toBeCloseTo(0.2);
  });
  it('clamps the second pane to its minimum', () => {
    expect(clampRatio(0.95, 1000 + HANDLE_PX, [200, 240])).toBeCloseTo(0.76);
  });
  it('splits proportionally to the minimums when both cannot fit', () => {
    expect(clampRatio(0.9, 300 + HANDLE_PX, [200, 200])).toBeCloseTo(0.5);
  });
  it('bounds to [0,1] when the size is unknown', () => {
    expect(clampRatio(1.7, 0, [200, 240])).toBe(1);
    expect(clampRatio(-1, 0, [200, 240])).toBe(0);
  });
});

describe('ratio persistence', () => {
  beforeEach(() => localStorage.clear());
  afterEach(() => vi.restoreAllMocks());

  it('round-trips through localStorage', () => {
    saveRatio('k', 0.37);
    expect(loadRatio('k')).toBeCloseTo(0.37);
  });
  it('rejects garbage and missing values', () => {
    expect(loadRatio('missing')).toBeNull();
    localStorage.setItem('crosswire.split.bad', 'NaN');
    expect(loadRatio('bad')).toBeNull();
    localStorage.setItem('crosswire.split.big', '4');
    expect(loadRatio('big')).toBeNull();
  });
  it('a throwing storage never throws', () => {
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('denied');
    });
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('denied');
    });
    expect(loadRatio('k')).toBeNull();
    expect(() => saveRatio('k', 0.5)).not.toThrow();
  });
});

describe('Splitter', () => {
  let cleanup: (() => void) | undefined;
  beforeEach(() => localStorage.clear());
  afterEach(() => {
    cleanup?.();
    cleanup = undefined;
    document.body.replaceChildren();
    vi.restoreAllMocks();
  });

  const pane = (text: string) => createRawSnippet(() => ({ render: () => `<div>${text}</div>` }));

  function render(props: Record<string, unknown>) {
    // jsdom has no layout: give every element a 1000 px wide, 600 px tall box.
    vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockReturnValue({
      x: 0, y: 0, left: 0, top: 0, width: 1000 + HANDLE_PX, height: 600 + HANDLE_PX,
      right: 1000 + HANDLE_PX, bottom: 600 + HANDLE_PX, toJSON: () => ({}),
    } as DOMRect);
    const target = document.createElement('div');
    document.body.append(target);
    const c = mount(Splitter, { target, props: { a: pane('A'), b: pane('B'), ...props } });
    flushSync();
    cleanup = () => unmount(c);
    return document.querySelector<HTMLElement>('[role="separator"]')!;
  }

  const key = (el: Element, k: string, shiftKey = false) => {
    el.dispatchEvent(new KeyboardEvent('keydown', { key: k, shiftKey, bubbles: true, cancelable: true }));
    flushSync();
  };

  it('renders both panes and a focusable separator', () => {
    const handle = render({ direction: 'row' });
    expect(document.body.textContent).toContain('A');
    expect(document.body.textContent).toContain('B');
    expect(handle.tabIndex).toBe(0);
    expect(handle.getAttribute('aria-orientation')).toBe('vertical');
    expect(handle.getAttribute('aria-valuenow')).toBe('50');
  });

  it('arrow keys resize and the ratio is persisted', () => {
    const handle = render({ direction: 'row', storageKey: 'history' });
    key(handle, 'ArrowRight', true);
    expect(handle.getAttribute('aria-valuenow')).toBe('60');
    expect(Number(localStorage.getItem('crosswire.split.history'))).toBeCloseTo(0.6);
  });

  it('keyboard resizing clamps to the minimum pane size', () => {
    const handle = render({ direction: 'row', min: [200, 240] });
    key(handle, 'Home');
    expect(handle.getAttribute('aria-valuenow')).toBe('20');
    for (let i = 0; i < 20; i++) key(handle, 'ArrowLeft', true);
    expect(handle.getAttribute('aria-valuenow')).toBe('20');
    key(handle, 'End');
    expect(handle.getAttribute('aria-valuenow')).toBe('76');
  });

  it('a column splitter uses ArrowUp/ArrowDown', () => {
    const handle = render({ direction: 'column', min: [100, 100] });
    expect(handle.getAttribute('aria-orientation')).toBe('horizontal');
    key(handle, 'ArrowDown', true);
    expect(handle.getAttribute('aria-valuenow')).toBe('60');
  });

  it('restores a persisted ratio on mount', () => {
    localStorage.setItem('crosswire.split.detail', '0.3');
    const handle = render({ direction: 'row', storageKey: 'detail' });
    expect(handle.getAttribute('aria-valuenow')).toBe('30');
  });
});
