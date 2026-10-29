import { test, expect } from '@playwright/test';

const ID = '01HZZZZZZZZZZZZZZZZZZZZZSHP';
const b64 = (s: string) => Buffer.from(s, 'utf8').toString('base64');
const HEAD = 'HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n';
const msg = (body: unknown) => ({
  kind: 'http1-raw', fidelity: 'raw', diagnostics: [],
  head: { type: 'http1-raw', bytes: b64(HEAD), parser_version: 1 }, body,
});

test('Preview mints a token and renders a sandboxed iframe on 127.0.0.1', async ({ page }) => {
  await page.route('**/api/v1/counts', (r) => r.fulfill({ status: 200, json: { exchanges: 1 } }));
  await page.route(/\/api\/v1\/exchanges\/[^/?]+$/, (r) =>
    r.fulfill({ status: 200, json: { id: ID, request: msg({ type: 'none' }), response: msg({ type: 'inline', truncated_at: 3 }), scope_state: 'in', state: 'committed', tags: [], notes: '' } }),
  );
  await page.route(/\/body\/req$/, (r) => r.fulfill({ status: 404, json: { error: { code: 'not_found', message: 'x' } } }));
  await page.route(/\/body\/resp$/, (r) =>
    r.fulfill({ status: 200, contentType: 'application/octet-stream', headers: { 'x-crosswire-length': '3' }, body: Buffer.from('abc') }),
  );
  await page.route(/\/api\/v1\/exchanges(\?.*)?$/, (r) =>
    r.fulfill({ status: 200, json: { items: [{ id: ID, created_at: 1_700_000_000_000, method: 'GET', host: 'a.example', path: '/', status: 200, resp_len: 3, state: 'committed', tags: [] }], next_cursor: null } }),
  );
  let minted = 0;
  await page.route('**/api/v1/exchanges/*/preview-token', (r) => {
    minted++;
    return r.fulfill({ status: 200, json: { token: 't', path: `/preview/${ID}?token=t`, expires_in: 60 } });
  });
  // The iframe target is a different origin with no server here; stub it.
  await page.route('http://127.0.0.1:*/preview/**', (r) => r.fulfill({ status: 200, contentType: 'text/html', body: '<p>x</p>' }));

  await page.goto('/');
  await page.getByTestId('history-row').first().click();
  await page.getByTestId('preview-button').click();

  const frame = page.getByTestId('preview-frame');
  await expect(frame).toBeVisible();
  expect(minted).toBe(1);
  const sandbox = (await frame.getAttribute('sandbox')) ?? '';
  expect(sandbox).not.toContain('allow-same-origin');
  expect(sandbox).not.toContain('allow-scripts');
  const src = (await frame.getAttribute('src')) ?? '';
  expect(src.startsWith('http://127.0.0.1:')).toBe(true);
  expect(src.endsWith(`/preview/${ID}?token=t`)).toBe(true);
});
