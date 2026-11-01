import { afterEach, describe, expect, it } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import KvTable from './KvTable.svelte';
import { parseForm, type KeyValue } from './pretty';

const enc = (s: string) => Uint8Array.from(s, (c) => c.charCodeAt(0));
const XSS = '<img src=x onerror=alert(1)>';

describe('KvTable', () => {
  let cleanup: (() => void) | undefined;
  afterEach(() => {
    cleanup?.();
    cleanup = undefined;
    document.body.replaceChildren();
  });

  function render(props: { rows: KeyValue[]; raw?: string; label: string; testid: string; total?: number; limit?: number }) {
    const target = document.createElement('div');
    document.body.append(target);
    const c = mount(KvTable, { target, props });
    flushSync();
    cleanup = () => unmount(c);
    return target;
  }

  it('renders keys, values and the raw string as text only', () => {
    const raw = `${encodeURIComponent(XSS)}=${encodeURIComponent(XSS)}&b=${XSS}`;
    const t = render({ rows: parseForm(enc(raw)), raw, testid: 'kv', label: 'Form' });
    expect(t.querySelector('img')).toBeNull();
    const keys = Array.from(t.querySelectorAll('[data-testid="kv-key"]')).map((c) => c.textContent);
    const values = Array.from(t.querySelectorAll('[data-testid="kv-value"]')).map((c) => c.textContent);
    expect(keys).toEqual([XSS, 'b']);
    expect(values).toEqual([XSS, XSS]);
    expect(t.querySelector('[data-testid="kv-raw"]')?.textContent).toBe(raw);
  });

  it('puts long values in a title and shows CR/LF as glyphs', () => {
    const long = 'v'.repeat(200);
    const t = render({ rows: [{ key: 'k', value: long }, { key: 'c', value: 'a\r\nb' }], testid: 'kv', label: 'Q' });
    const cells = t.querySelectorAll('[data-testid="kv-value"]');
    expect(cells[0].getAttribute('title')).toBe(long);
    expect(cells[1].getAttribute('title')).toBeNull();
    expect(cells[1].textContent).toBe('a␍␊b');
    expect(t.querySelector('[data-testid="kv-raw"]')).toBeNull();
  });

  it('renders at most 1000 rows and says how many there are', () => {
    const rows = Array.from({ length: 5000 }, (_, i) => ({ key: `k${i}`, value: `<b>${i}</b>` }));
    const t = render({ rows, testid: 'kv', label: 'Form' });
    expect(t.querySelectorAll('[data-testid="kv-key"]')).toHaveLength(1000);
    expect(t.querySelector('b')).toBeNull();
    expect(t.querySelector('[data-testid="kv-more"]')?.textContent).toBe('Showing 1000 of 5000 fields; Raw shows all.');
  });

  it('takes the true total when the rows were already capped', () => {
    const rows = Array.from({ length: 1001 }, (_, i) => ({ key: `k${i}`, value: '' }));
    const t = render({ rows, total: 699050, testid: 'kv', label: 'Form' });
    expect(t.querySelectorAll('[data-testid="kv-key"]')).toHaveLength(1000);
    expect(t.querySelector('[data-testid="kv-more"]')?.textContent).toBe('Showing 1000 of 699050 fields; Raw shows all.');
  });

  it('shows no note at or under the limit and clips a huge raw string', () => {
    const t = render({ rows: [{ key: 'a', value: '1' }], raw: 'x'.repeat(200_000), testid: 'kv', label: 'F' });
    expect(t.querySelector('[data-testid="kv-more"]')).toBeNull();
    expect(t.querySelector('[data-testid="kv-raw"]')!.textContent!.length).toBeLessThan(70_000);
    expect(t.querySelector('[data-testid="kv-raw-clipped"]')).not.toBeNull();
  });
});
