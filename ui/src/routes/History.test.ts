import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import History from './History.svelte';
import { keymap } from '../lib/keymap';

const XSS = '<img src=x onerror=alert(1)>';
const SCRIPT = '"><script>alert(1)</script>';

const ITEMS = [
  { id: 'a', created_at: 1_700_000_300_000, method: 'GET', host: 'one.test', path: '/login', status: 200, resp_len: 5, state: 'committed', tags: ['auth'] },
  { id: 'b', created_at: 1_700_000_200_000, method: 'POST', host: XSS, path: `/${SCRIPT}`, status: 500, resp_len: 9, state: 'committed', tags: [SCRIPT] },
  { id: 'c', created_at: 1_700_000_100_000, method: 'GET', host: 'three.test', path: '/x', status: 404, resp_len: 1, state: 'committed', tags: ['idor'] },
];

class FakeSocket {
  readyState = 0;
  onopen: (() => void) | null = null;
  onmessage: (() => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  close() {}
  send() {}
  addEventListener() {}
  removeEventListener() {}
}

describe('History view', () => {
  let cleanup: (() => void) | undefined;

  beforeEach(() => {
    localStorage.clear();
    vi.stubGlobal('WebSocket', FakeSocket);
    vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} });
    vi.stubGlobal(
      'fetch',
      vi.fn(async (url: string) => {
        const body = String(url).startsWith('/api/v1/exchanges') ? { items: ITEMS, next_cursor: null } : {};
        return new Response(JSON.stringify(body), { status: 200 });
      }),
    );
  });
  afterEach(() => {
    cleanup?.();
    cleanup = undefined;
    document.body.replaceChildren();
    vi.unstubAllGlobals();
  });

  async function render(): Promise<HTMLElement> {
    const target = document.createElement('div');
    document.body.append(target);
    const c = mount(History, { target });
    cleanup = () => unmount(c);
    for (let i = 0; i < 20 && target.querySelectorAll('[data-testid="history-row"]').length === 0; i++) {
      await new Promise((r) => setTimeout(r, 10));
      flushSync();
    }
    await new Promise((r) => setTimeout(r, 20));
    flushSync();
    return target;
  }

  const rows = (t: HTMLElement) => Array.from(t.querySelectorAll<HTMLElement>('[data-testid="history-row"]'));
  const search = (t: HTMLElement) => t.querySelector<HTMLInputElement>('[data-testid="history-search"]')!;
  const key = (el: Element, k: string) => {
    el.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true }));
    flushSync();
  };
  function type(t: HTMLElement, q: string) {
    const box = search(t);
    box.value = q;
    box.dispatchEvent(new Event('input', { bubbles: true }));
    flushSync();
  }

  it('renders hostile host, path and tags as text and matches them as text', async () => {
    const t = await render();
    expect(rows(t)).toHaveLength(3);
    expect(t.querySelector('img')).toBeNull();
    expect(t.querySelector('script')).toBeNull();
    expect(rows(t)[1].textContent).toContain(XSS);
    expect(rows(t)[1].querySelector('td.tags')!.textContent).toContain(SCRIPT);

    type(t, '<img src=x');
    expect(rows(t)).toHaveLength(1);
    expect(t.querySelector('[data-testid="history-count"]')!.textContent).toContain('1 of 3 loaded match');
    expect(t.querySelector('img')).toBeNull();
  });

  it('filters by host, path and tag; Esc clears and returns focus to the list', async () => {
    const t = await render();
    type(t, 'IDOR');
    expect(rows(t).map((r) => r.dataset.id)).toEqual(['c']);
    type(t, 'login');
    expect(rows(t).map((r) => r.dataset.id)).toEqual(['a']);
    type(t, 'nothing-matches');
    expect(t.querySelector('[data-testid="history-empty"]')).not.toBeNull();

    search(t).focus();
    key(search(t), 'Escape');
    await new Promise((r) => setTimeout(r, 0));
    flushSync();
    expect(search(t).value).toBe('');
    expect(rows(t)).toHaveLength(3);
    expect(rows(t).includes(document.activeElement as HTMLElement)).toBe(true);
  });

  it('/ focuses the search box, but not while typing in a field', async () => {
    // `/` is the keymap's focus-search action; the Shell owns the one keydown
    // dispatch. Dispatch with no route scope (null), so the History scope can
    // only come from the view's own data-keyscope root.
    const onKey = (e: KeyboardEvent) => keymap.dispatch(e, null);
    window.addEventListener('keydown', onKey);
    try {
      const t = await render();
      const first = rows(t)[0];
      first.focus();
      const slash = new KeyboardEvent('keydown', { key: '/', code: 'Slash', bubbles: true, cancelable: true });
      first.dispatchEvent(slash);
      expect(document.activeElement).toBe(search(t));
      expect(slash.defaultPrevented).toBe(true);

      // Typing '/' in the search box itself must stay text.
      const ev = new KeyboardEvent('keydown', { key: '/', code: 'Slash', bubbles: true, cancelable: true });
      search(t).dispatchEvent(ev);
      expect(ev.defaultPrevented).toBe(false);
    } finally {
      window.removeEventListener('keydown', onKey);
    }
  });

  it('j/k and arrows move a roving focus; Enter opens; Space checks', async () => {
    const t = await render();
    const r = rows(t);
    expect(r.map((x) => x.tabIndex)).toEqual([0, -1, -1]);
    r[0].focus();
    key(r[0], 'j');
    expect(document.activeElement).toBe(r[1]);
    key(r[1], 'ArrowDown');
    expect(document.activeElement).toBe(r[2]);
    key(r[2], 'j');
    expect(document.activeElement).toBe(r[2]);
    key(r[2], 'k');
    expect(document.activeElement).toBe(r[1]);
    key(r[1], 'ArrowUp');
    expect(document.activeElement).toBe(r[0]);
    expect(rows(t).map((x) => x.tabIndex)).toEqual([0, -1, -1]);

    key(r[1], ' ');
    expect(r[1].classList.contains('checked')).toBe(true);
    key(r[1], 'Enter');
    expect(r[1].classList.contains('selected')).toBe(true);
  });

  it('ignores j/k typed into the filter fields and the search box', async () => {
    const t = await render();
    const box = search(t);
    const ev = new KeyboardEvent('keydown', { key: 'j', bubbles: true, cancelable: true });
    box.dispatchEvent(ev);
    expect(ev.defaultPrevented).toBe(false);
    const host = t.querySelector<HTMLInputElement>('[data-testid="filter-host"]')!;
    const ev2 = new KeyboardEvent('keydown', { key: 'k', bubbles: true, cancelable: true });
    host.dispatchEvent(ev2);
    expect(ev2.defaultPrevented).toBe(false);
  });

  it('hides a column from header and cells, persists it, and restores it on remount', async () => {
    localStorage.setItem('crosswire.history.columns', '["time","method","status","len","state"]');
    const t = await render();
    expect(t.querySelector('td.tags')).toBeNull();
    expect(Array.from(t.querySelectorAll('th')).some((h) => h.textContent === 'Tags')).toBe(false);
    expect(t.querySelector('td.time')).not.toBeNull();

    t.querySelector<HTMLElement>('[data-testid="column-picker"]')!.click();
    flushSync();
    t.ownerDocument.querySelector<HTMLElement>('[data-testid="column-time"]')!.click();
    flushSync();
    expect(t.querySelector('td.time')).toBeNull();
    expect(localStorage.getItem('crosswire.history.columns')).toBe('["method","status","len","state"]');
  });

  it('Escape in a detail field keeps focus there; Escape elsewhere in the detail returns to the list', async () => {
    const t = await render();
    const body = t.querySelector<HTMLElement>('.detail-body')!;
    const notes = document.createElement('textarea');
    body.append(notes);
    notes.focus();
    key(notes, 'Escape');
    expect(document.activeElement).toBe(notes);

    const btn = document.createElement('button');
    body.append(btn);
    btn.focus();
    key(btn, 'Escape');
    expect(rows(t).includes(document.activeElement as HTMLElement)).toBe(true);
  });

  it('drops hidden rows from the selection when the search changes', async () => {
    const t = await render();
    const boxes = () => t.querySelectorAll<HTMLInputElement>('tbody input[type="checkbox"]');
    for (const b of boxes()) b.click();
    flushSync();
    expect(t.querySelector('[data-testid="history-count"]')!.textContent).toContain('3 selected');
    type(t, 'idor');
    flushSync();
    expect(t.querySelector('[data-testid="history-count"]')!.textContent).toContain('1 selected');
    type(t, '');
    flushSync();
    expect(t.querySelector('[data-testid="history-count"]')!.textContent).toContain('1 selected');
  });

  it('shows every column by default, with Tags cells present', async () => {
    const t = await render();
    expect(t.querySelectorAll('td.tags')).toHaveLength(3);
    expect(t.querySelector('td.time')!.getAttribute('title')).not.toBe('');
  });
});
