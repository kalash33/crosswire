import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import Tabs from './Tabs.svelte';

const TABS = [
  { id: 'headers', label: 'Headers' },
  { id: 'raw', label: 'Raw' },
  { id: 'hex', label: 'Hex' },
];

describe('Tabs', () => {
  let cleanup: (() => void) | undefined;
  afterEach(() => {
    cleanup?.();
    cleanup = undefined;
    document.body.replaceChildren();
  });

  function render(active: string, onselect = vi.fn()) {
    const c = mount(Tabs, { target: document.body, props: { tabs: TABS, active, onselect } });
    flushSync();
    cleanup = () => unmount(c);
    const tabs = Array.from(document.querySelectorAll<HTMLButtonElement>('[role="tab"]'));
    const list = document.querySelector<HTMLElement>('[role="tablist"]')!;
    return { tabs, list, onselect };
  }
  const key = (el: Element, k: string) =>
    el.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true }));

  it('only the active tab is in the tab order', () => {
    const { tabs } = render('raw');
    expect(tabs.map((t) => t.tabIndex)).toEqual([-1, 0, -1]);
    expect(tabs[1].getAttribute('aria-selected')).toBe('true');
  });

  it('falls back to the first tab when active matches none', () => {
    const { tabs } = render('nope');
    expect(tabs.map((t) => t.tabIndex)).toEqual([0, -1, -1]);
  });

  it('arrow keys wrap from the fallback position', () => {
    const { list, onselect } = render('nope');
    key(list, 'ArrowLeft');
    expect(onselect).toHaveBeenLastCalledWith('hex');
    key(list, 'ArrowRight');
    expect(onselect).toHaveBeenLastCalledWith('raw');
  });

  it('ArrowLeft from the first tab wraps to the last; Home/End jump', () => {
    const { list, onselect } = render('headers');
    key(list, 'ArrowLeft');
    expect(onselect).toHaveBeenLastCalledWith('hex');
    key(list, 'End');
    expect(onselect).toHaveBeenLastCalledWith('hex');
    key(list, 'Home');
    expect(onselect).toHaveBeenLastCalledWith('headers');
  });
});
