import { test, expect, type Page } from '@playwright/test';

// No backend: Playwright serves the built bundle via `vite preview` and we mock
// every API call with page.route. The app authenticates off a 200 from
// /api/v1/counts (session.probe), then History loads /api/v1/exchanges.

const COUNTS = {
  bytes_inline: 0, bytes_packs: 0, committed: 0, error: 0, exchanges: 2,
  history: 2, intruder: 0, markers: 0, repeater: 0,
};

const SHOP_ID = '01HZZZZZZZZZZZZZZZZZZZZZSHP';
const API_ID = '01HZZZZZZZZZZZZZZZZZZZZZAPI';

function row(id: string, host: string, path: string, status: number) {
  return {
    id,
    created_at: 1_700_000_000_000,
    method: 'GET',
    host,
    path,
    status,
    resp_len: 15,
    state: 'committed',
    tags: ['demo'],
  };
}

const ALL_ROWS = [row(SHOP_ID, 'shop.example', '/cart', 200), row(API_ID, 'api.other', '/v1/ping', 204)];

const b64 = (s: string) => Buffer.from(s, 'utf8').toString('base64');
const RESP_HEAD = 'HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 15\r\n\r\n';
const REQ_HEAD = 'GET /cart HTTP/1.1\r\nHost: shop.example\r\n\r\n';
const RESP_BODY = 'HELLO-RESP-BODY'; // 15 printable ASCII bytes

function exchangeJson(id: string) {
  return {
    id,
    request: {
      kind: 'http1-raw',
      fidelity: 'raw',
      head: { type: 'http1-raw', bytes: b64(REQ_HEAD), parser_version: 1 },
      diagnostics: [],
      body: { type: 'none' },
    },
    response: {
      kind: 'http1-raw',
      fidelity: 'raw',
      head: { type: 'http1-raw', bytes: b64(RESP_HEAD), parser_version: 1 },
      diagnostics: [],
      body: { type: 'inline', truncated_at: 15 },
    },
    scope_state: 'in',
    state: 'committed',
    tags: ['demo'],
    notes: '',
  };
}

async function mockApi(page: Page) {
  await page.route('**/api/v1/counts', (route) => route.fulfill({ status: 200, json: COUNTS }));

  // Detail: GET /exchanges/{id}
  await page.route(/\/api\/v1\/exchanges\/[^/?]+$/, (route) =>
    route.fulfill({ status: 200, json: exchangeJson(SHOP_ID) }),
  );

  // Body: req has none (404), resp returns raw octet-stream bytes + headers.
  await page.route(/\/api\/v1\/exchanges\/[^/]+\/body\/req$/, (route) =>
    route.fulfill({ status: 404, json: { error: { code: 'not_found', message: 'no such body' } } }),
  );
  await page.route(/\/api\/v1\/exchanges\/[^/]+\/body\/resp$/, (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/octet-stream',
      headers: { 'x-crosswire-length': String(RESP_BODY.length), 'x-crosswire-truncated-at': '15' },
      body: Buffer.from(RESP_BODY, 'utf8'),
    }),
  );

  // List: GET /exchanges — narrow by host / host_contains when present.
  await page.route(/\/api\/v1\/exchanges(\?.*)?$/, (route) => {
    const url = new URL(route.request().url());
    const host = url.searchParams.get('host');
    const contains = url.searchParams.get('host_contains');
    let items = ALL_ROWS;
    if (host) items = items.filter((r) => r.host === host);
    if (contains) items = items.filter((r) => r.host.includes(contains));
    route.fulfill({ status: 200, json: { items, next_cursor: null } });
  });
}

test.beforeEach(async ({ page }) => mockApi(page));

test('history loads a seeded page, a host filter narrows it, and a row opens byte-exact detail', async ({
  page,
}) => {
  await page.goto('/');

  await expect(page.getByTestId('app-shell')).toBeVisible();
  await expect(page.getByTestId('history-view')).toBeVisible();

  // Seeded page: both rows present.
  await expect(page.getByTestId('history-row')).toHaveCount(2);

  // Host filter narrows the list and the request carries the `host` param.
  const filtered = page.waitForRequest(
    (r) => r.url().includes('/api/v1/exchanges') && r.url().includes('host=shop.example'),
  );
  await page.getByTestId('filter-host').fill('shop.example');
  await page.getByTestId('filter-apply').click();
  await filtered;
  await expect(page.getByTestId('history-row')).toHaveCount(1);
  await expect(page.getByTestId('history-list')).toContainText('shop.example');
  await expect(page.getByTestId('history-list')).not.toContainText('api.other');

  // Selecting the row opens the detail.
  await page.getByTestId('history-row').first().click();
  await expect(page.getByTestId('exchange-detail')).toBeVisible();

  // The response bytes render byte-exact through the ByteEditor (CodeMirror).
  const respBody = page.getByTestId('detail-resp-body');
  await expect(respBody).toBeVisible();
  await expect(respBody.locator('.cm-content')).toContainText(RESP_BODY);

  // Truncation indicator surfaced via the editor's diagnostic banner.
  await expect(respBody.getByTestId('diagnostic-banner')).toContainText('truncated at 15');

  // The response head is rendered byte-exact (raw head bytes).
  await expect(page.getByTestId('detail-resp-head')).toHaveAttribute('data-exact', 'true');
});
