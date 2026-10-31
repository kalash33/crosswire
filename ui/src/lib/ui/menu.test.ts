import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import Menu from './Menu.svelte';

type Item = { label: string; onselect: () => void; danger?: boolean };

describe('Menu', () => {
  let anchor: HTMLButtonElement;
  let cleanup: (() => void) | undefined;

  beforeEach(() => {
    anchor = document.createElement('button');
    anchor.textContent = 'More';
    document.body.append(anchor);
    anchor.focus();
  });
  afterEach(() => {
    cleanup?.();
    cleanup = undefined;
    document.body.replaceChildren();
  });

  function open(items: Item[], onclose = vi.fn()) {
    const target = document.createElement('div');
    document.body.append(target);
    const c = mount(Menu, { target, props: { items, open: true, anchor, onclose } });
    flushSync();
    cleanup = () => unmount(c);
    return { onclose, menu: document.querySelector<HTMLElement>('[role="menu"]')! };
  }

  const key = (el: Element, k: string) =>
    el.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true }));
  const items = () => Array.from(document.querySelectorAll<HTMLButtonElement>('[role="menuitem"]'));

  it('renders items as buttons and focuses the first', () => {
    open([
      { label: 'Copy URL', onselect: vi.fn() },
      { label: 'Copy as curl', onselect: vi.fn() },
    ]);
    expect(items().map((b) => b.tagName)).toEqual(['BUTTON', 'BUTTON']);
    expect(document.activeElement).toBe(items()[0]);
  });

  it('ArrowDown/ArrowUp move focus and wrap; Home/End jump', () => {
    const { menu } = open([
      { label: 'A', onselect: vi.fn() },
      { label: 'B', onselect: vi.fn() },
      { label: 'C', onselect: vi.fn() },
    ]);
    key(menu, 'ArrowDown');
    expect(document.activeElement).toBe(items()[1]);
    key(menu, 'ArrowDown');
    key(menu, 'ArrowDown');
    expect(document.activeElement).toBe(items()[0]);
    key(menu, 'ArrowUp');
    expect(document.activeElement).toBe(items()[2]);
    key(menu, 'Home');
    expect(document.activeElement).toBe(items()[0]);
    key(menu, 'End');
    expect(document.activeElement).toBe(items()[2]);
  });

  it('Enter selects the focused item, closes and returns focus to the anchor', () => {
    const b = vi.fn();
    const { menu, onclose } = open([
      { label: 'A', onselect: vi.fn() },
      { label: 'B', onselect: b },
    ]);
    key(menu, 'ArrowDown');
    key(menu, 'Enter');
    expect(b).toHaveBeenCalledOnce();
    expect(onclose).toHaveBeenCalledOnce();
    expect(document.activeElement).toBe(anchor);
  });

  it('Space selects too', () => {
    const a = vi.fn();
    const { menu } = open([{ label: 'A', onselect: a }]);
    key(menu, ' ');
    expect(a).toHaveBeenCalledOnce();
  });

  it('Escape closes without selecting and restores focus', () => {
    const a = vi.fn();
    const { menu, onclose } = open([{ label: 'A', onselect: a }]);
    key(menu, 'Escape');
    expect(onclose).toHaveBeenCalledOnce();
    expect(a).not.toHaveBeenCalled();
    expect(document.activeElement).toBe(anchor);
  });

  it('a click outside closes; a click inside does not', () => {
    const { menu, onclose } = open([{ label: 'A', onselect: vi.fn() }]);
    menu.dispatchEvent(new MouseEvent('pointerdown', { bubbles: true }));
    expect(onclose).not.toHaveBeenCalled();
    document.body.dispatchEvent(new MouseEvent('pointerdown', { bubbles: true }));
    expect(onclose).toHaveBeenCalledOnce();
  });

  it('renders labels as text, never markup', () => {
    open([{ label: '<img src=x onerror=alert(1)>', onselect: vi.fn() }]);
    expect(items()[0].textContent).toContain('<img src=x onerror=alert(1)>');
    expect(document.querySelector('img')).toBeNull();
  });
});
