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

  function render(props: { rows: KeyValue[]; raw?: string; label: string; testid: string }) {
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
});
