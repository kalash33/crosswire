import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import SendHistory from './SendHistory.svelte';
import DiffView from './DiffView.svelte';
import type { SendRecord } from '../stores/repeater.svelte';
import { DIFF_LINE_SHOW } from '../diff/lines';

const XSS = '<img src=x onerror=alert(1)>';
const conn = { host: XSS, port: 80, tls: false, reuse: false, follow: 'off' as const };

function rec(over: Partial<SendRecord> = {}): SendRecord {
  return {
    at: '2026-11-01T10:20:30.000Z',
    request_sha256: 'ab'.repeat(32),
    request_len: 10,
    conn,
    mode: 'raw',
    ...over,
  };
}

let cleanup: (() => void) | undefined;
afterEach(() => {
  cleanup?.();
  cleanup = undefined;
  document.body.replaceChildren();
  localStorage.clear();
});

function render<P extends Record<string, unknown>>(component: unknown, props: P): HTMLElement {
  const target = document.createElement('div');
  document.body.append(target);
  const c = mount(component as typeof SendHistory, { target, props: props as never });
  flushSync();
  cleanup = () => unmount(c);
  return target;
}

describe('SendHistory', () => {
  const handlers = () => ({ onrestore: vi.fn(), onclear: vi.fn(), oncompare: vi.fn() });

  it('renders rows with status, size and TTFB, and hostile text stays text', () => {
    const h = handlers();
    const t = render(SendHistory, {
      history: [rec({ status: 200, size: 2048, ttfb_ms: 31, exchange_id: 'A' }), rec({ error: XSS })],
      shown: 0,
      notice: XSS,
      ...h,
    });
    expect(t.querySelector('img')).toBeNull();
    const rows = t.querySelectorAll('[data-testid="send-history-row"]');
    expect(rows).toHaveLength(2);
    expect(rows[0].querySelector('[data-testid="send-history-status"]')?.textContent).toBe('200');
    expect(rows[0].querySelector('[data-testid="send-history-size"]')?.textContent).toBe('2.0 KB');
    expect(rows[0].querySelector('[data-testid="send-history-ttfb"]')?.textContent).toBe('31 ms');
    expect(rows[0].getAttribute('aria-current')).toBe('true');
    expect(rows[1].textContent).toContain(XSS);
    expect(rows[1].getAttribute('title')).toContain(XSS);
    expect(t.querySelector('[data-testid="send-history-notice"]')?.textContent).toBe(XSS);
    (rows[1] as HTMLButtonElement).click();
    expect(h.onrestore).toHaveBeenCalledWith(1);
  });

  it('compare is enabled only when allowed; clear and collapse work and collapse is remembered', () => {
    const h = handlers();
    const t = render(SendHistory, { history: [rec()], canCompare: false, ...h });
    const cmp = t.querySelector<HTMLButtonElement>('[data-testid="compare-previous"]')!;
    expect(cmp.disabled).toBe(true);
    t.querySelector<HTMLButtonElement>('[data-testid="send-history-clear"]')!.click();
    expect(h.onclear).toHaveBeenCalled();
    t.querySelector<HTMLButtonElement>('[data-testid="send-history-toggle"]')!.click();
    flushSync();
    expect(t.querySelectorAll('[data-testid="send-history-row"]')).toHaveLength(0);
    expect(localStorage.getItem('crosswire.repeater.history-collapsed')).toBe('1');
    cleanup?.();
    const t2 = render(SendHistory, { history: [rec()], canCompare: true, ...handlers() });
    expect(t2.querySelectorAll('[data-testid="send-history-row"]')).toHaveLength(0);
    expect(t2.querySelector<HTMLButtonElement>('[data-testid="compare-previous"]')!.disabled).toBe(false);
  });
});

describe('DiffView', () => {
  it('renders added and removed lines as text only', () => {
    const t = render(DiffView, {
      olderText: `HTTP/1.1 200 OK\r\nX: ${XSS}\r\n\r\nsame`,
      newerText: `HTTP/1.1 500 Oops\r\nX: ${XSS}\r\n\r\nsame\n<script>alert(1)</script>`,
      olderLabel: XSS,
      newerLabel: 'b',
      onclose: vi.fn(),
    });
    expect(t.querySelector('img')).toBeNull();
    expect(t.querySelector('script')).toBeNull();
    const lines = Array.from(t.querySelectorAll('[data-testid="diff-line"]')).map((l) => [
      l.getAttribute('data-kind'),
      l.querySelector('[data-testid="diff-text"]')?.textContent,
    ]);
    expect(lines).toEqual([
      ['del', 'HTTP/1.1 200 OK'],
      ['add', 'HTTP/1.1 500 Oops'],
      ['same', `X: ${XSS}`],
      ['same', ''],
      ['same', 'same'],
      ['add', '<script>alert(1)</script>'],
    ]);
    expect(t.querySelector('[data-testid="diff-older"]')?.textContent).toBe(XSS);
    expect(t.querySelector('[data-testid="diff-summary"]')?.textContent?.replace(/\s+/g, ' ').trim()).toBe('+2 -1');
  });

  it('hides unchanged runs behind a fold that expands, and the toggle shows all', () => {
    const body = Array.from({ length: 30 }, (_, i) => `line ${i}`);
    const t = render(DiffView, {
      olderText: body.join('\n'),
      newerText: [...body.slice(0, 15), 'changed', ...body.slice(16)].join('\n'),
      olderLabel: 'a',
      newerLabel: 'b',
      onclose: vi.fn(),
    });
    const folds = () => t.querySelectorAll('[data-testid="diff-fold"]');
    expect(folds()).toHaveLength(2);
    expect(folds()[0].textContent).toBe('… 12 unchanged lines …');
    (folds()[0] as HTMLButtonElement).click();
    flushSync();
    expect(folds()).toHaveLength(1);
    t.querySelector<HTMLInputElement>('[data-testid="diff-hide-unchanged"]')!.click();
    flushSync();
    expect(folds()).toHaveLength(0);
    expect(t.querySelectorAll('[data-testid="diff-line"]')).toHaveLength(31);
  });

  it('caps a long rendered line at 4 KiB but still diffs the whole line', () => {
    const long = 'a'.repeat(10 * 1024);
    const t = render(DiffView, {
      olderText: `${long}x`,
      newerText: `${long}y`,
      olderLabel: 'a',
      newerLabel: 'b',
      onclose: vi.fn(),
    });
    const texts = Array.from(t.querySelectorAll('[data-testid="diff-text"]'));
    expect(texts.map((x) => x.getAttribute('data-kind') ?? x.parentElement?.getAttribute('data-kind'))).toEqual(['del', 'add']);
    for (const x of texts) {
      expect(x.textContent!.startsWith('a'.repeat(DIFF_LINE_SHOW))).toBe(true);
      expect(x.textContent!.length).toBeLessThan(DIFF_LINE_SHOW + 40);
      expect(x.textContent).toContain(`… (${10 * 1024 + 1 - DIFF_LINE_SHOW} more chars)`);
    }
  });

  it('shows the too-large state when told the responses are over the byte cap', () => {
    const t = render(DiffView, { olderText: '', newerText: '', tooLarge: true, olderLabel: 'a', newerLabel: 'b', onclose: vi.fn() });
    expect(t.querySelector('[data-testid="diff-too-large"]')).not.toBeNull();
  });

  it('says when the responses are too large to diff', () => {
    const t = render(DiffView, {
      olderText: 'x\n'.repeat(20001),
      newerText: 'y',
      olderLabel: 'a',
      newerLabel: 'b',
      onclose: vi.fn(),
    });
    expect(t.querySelector('[data-testid="diff-too-large"]')).not.toBeNull();
    expect(t.querySelectorAll('[data-testid="diff-line"]')).toHaveLength(0);
  });
});
