import { test, expect, type Page } from '@playwright/test';

// No backend: mock the API and capture clipboard writes through an injected
// navigator.clipboard stub (deterministic in headless; no permissions).

const ID = '01HZZZZZZZZZZZZZZZZZZZZZSHP';
const COUNTS = {
  bytes_inline: 0, bytes_packs: 0, committed: 0, error: 0, exchanges: 1,
  history: 1, intruder: 0, markers: 0, repeater: 0,
};
const b64 = (s: string) => Buffer.from(s, 'utf8').toString('base64');

async function setup(page: Page) {
  const calls: string[] = [];
  await page.addInitScript(() => {
    (window as unknown as { __clip: string[] }).__clip = [];
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: { writeText: async (t: string) => void (window as unknown as { __clip: string[] }).__clip.push(t) },
    });
  });
  await page.route('**/api/v1/counts', (r) => r.fulfill({ status: 200, json: COUNTS }));
  await page.route(/\/api\/v1\/exchanges\/[^/?]+$/, (r) =>
    r.fulfill({
      status: 200,
      json: {
        id: ID,
        request: {
          kind: 'http1-raw', fidelity: 'raw',
          head: { type: 'http1-raw', bytes: b64('GET /cart HTTP/1.1\r\nHost: shop.example\r\n\r\n'), parser_version: 1 },
          diagnostics: [], body: { type: 'none' },
        },
        response: null, scope_state: 'in', state: 'committed', tags: [], notes: '',
      },
    }),
  );
  await page.route(/\/api\/v1\/exchanges\/[^/]+\/body\/req$/, (r) =>
    r.fulfill({ status: 404, json: { error: { code: 'not_found', message: 'none' } } }),
  );
  await page.route(/\/api\/v1\/exchanges\/[^/]+\/curl\?/, (r) => {
    const u = new URL(r.request().url());
    calls.push(`curl:${u.searchParams.get('redacted')}:${u.searchParams.get('shell')}`);
    r.fulfill({
      status: 200, contentType: 'text/plain',
      body: `curl redacted=${u.searchParams.get('redacted')} shell=${u.searchParams.get('shell')}`,
    });
  });
  await page.route(/\/api\/v1\/exchanges\/[^/]+\/raw\?/, (r) => {
    const u = new URL(r.request().url());
    calls.push(`raw:${u.searchParams.get('redacted')}`);
    r.fulfill({
      status: 200, contentType: 'application/octet-stream',
      body: Buffer.from(`RAW redacted=${u.searchParams.get('redacted')}`),
    });
  });
  await page.route(/\/api\/v1\/exchanges(\?.*)?$/, (r) =>
    r.fulfill({
      status: 200,
      json: {
        items: [{ id: ID, created_at: 1_700_000_000_000, method: 'GET', host: 'shop.example',
          path: '/cart', status: 200, resp_len: 0, state: 'committed', tags: [] }],
        next_cursor: null,
      },
    }),
  );
  return calls;
}

const clip = (page: Page) => page.evaluate(() => (window as unknown as { __clip: string[] }).__clip);

test('copy buttons call the audited routes with redacted=false/true', async ({ page }) => {
  const calls = await setup(page);
  await page.goto('/');
  await page.getByTestId('history-row').first().click();
  await expect(page.getByTestId('copy-menu')).toBeVisible();

  // U1: the copy items live in the detail's Copy dropdown (title line), which
  // closes after each pick, so open it before every item.
  const pick = async (item: string) => {
    await page.getByTestId('copy-menu').click();
    await page.getByTestId(item).click();
  };
  await pick('copy-curl');
  await expect(page.getByTestId('copy-status')).toContainText('Copied curl.');
  await pick('copy-curl-redacted');
  await expect(page.getByTestId('copy-status')).toContainText('Copied curl (redacted).');
  await pick('copy-request-raw-redacted');
  await expect(page.getByTestId('copy-status')).toContainText('Copied raw (redacted).');

  const c = await clip(page);
  expect(c[0]).toMatch(/^curl redacted=false shell=(sh|pwsh)$/);
  expect(c[1]).toMatch(/^curl redacted=true shell=(sh|pwsh)$/);
  expect(c[2]).toBe('RAW redacted=true');
  expect(calls[0]).toMatch(/^curl:false:/);
  expect(calls[1]).toMatch(/^curl:true:/);
  expect(calls[2]).toBe('raw:true');
});

test('the Burp chord copies unredacted curl; ctrl+shift+s copies unredacted raw', async ({ page }) => {
  await setup(page);
  await page.goto('/');
  await page.getByTestId('history-row').first().click();
  await expect(page.getByTestId('copy-menu')).toBeVisible();

  await page.keyboard.press('Control+Shift+C');
  await expect(page.getByTestId('copy-status')).toContainText('Copied curl.');
  await page.keyboard.press('Control+Shift+S');
  await expect(page.getByTestId('copy-status')).toContainText('Copied raw.');

  const c = await clip(page);
  expect(c[0]).toMatch(/^curl redacted=false /);
  expect(c[1]).toBe('RAW redacted=false');
});
