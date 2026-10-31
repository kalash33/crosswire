import { afterEach, describe, expect, it } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import HeadersTable from './HeadersTable.svelte';
import DiagnosticsList from './DiagnosticsList.svelte';

const enc = (s: string) => Uint8Array.from(s, (c) => c.charCodeAt(0));
const XSS = '<img src=x onerror=alert(1)>';

describe('HeadersTable', () => {
  let cleanup: (() => void) | undefined;
  afterEach(() => {
    cleanup?.();
    cleanup = undefined;
    document.body.replaceChildren();
  });

  function render(props: Record<string, unknown>) {
    const target = document.createElement('div');
    document.body.append(target);
    const c = mount(HeadersTable, { target, props });
    flushSync();
    cleanup = () => unmount(c);
    return target;
  }

  it('renders markup in names, values and the start line as text', () => {
    const t = render({
      bytes: enc(`GET /${XSS} HTTP/1.1\r\nX-${XSS}: ${XSS}\r\nno colon ${XSS}\r\n\r\n`),
    });
    expect(t.querySelector('img')).toBeNull();
    const cells = Array.from(t.querySelectorAll('td, th[scope="row"]')).map((c) => c.textContent);
    expect(cells).toContain(XSS);
    expect(cells).toContain(`X-${XSS}`);
    expect(t.querySelector('[data-testid="headers-start"]')?.textContent).toBe(`GET /${XSS} HTTP/1.1`);
    expect(t.querySelector('[data-testid="headers-malformed"]')?.textContent).toContain(`no colon ${XSS}`);
  });

  it('renders structured (normalized) headers and never emits CR or LF', () => {
    const t = render({ structured: [{ name: 'A', value: 'x\r\ny' }] });
    expect(t.querySelector('img')).toBeNull();
    expect(t.textContent).not.toMatch(/[\r\n]x|x[\r\n]/);
    expect(t.querySelector('td')?.textContent).toBe('x␍␊y');
  });
});

describe('DiagnosticsList', () => {
  it('groups by code with a count and keeps spans inside a collapsible', () => {
    const target = document.createElement('div');
    document.body.append(target);
    const c = mount(DiagnosticsList, {
      target,
      props: {
        testid: 'd',
        diagnostics: [
          { code: 'non_canonical_header_case', severity: 'info', span: { start: 1, end: 2 } },
          { code: XSS, severity: 'error', span: { start: 3, end: 4 } },
          { code: 'non_canonical_header_case', severity: 'warn', span: { start: 5, end: 6 } },
        ],
      },
    });
    flushSync();
    const groups = Array.from(target.querySelectorAll<HTMLElement>('[data-testid="diag-group"]'));
    expect(groups.map((g) => [g.dataset.code, g.dataset.count])).toEqual([
      ['non_canonical_header_case', '2'],
      [XSS, '1'],
    ]);
    expect(groups[0].tagName).toBe('DETAILS');
    expect(groups[0].className).toContain('sev-warn');
    expect(groups[1].className).toContain('sev-error');
    expect(groups[0].querySelectorAll('li')).toHaveLength(2);
    expect(target.querySelector('img')).toBeNull();
    unmount(c);
  });
});
