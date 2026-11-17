import { test, expect, type Page } from '@playwright/test';

// U2 D3: History search, keyboard navigation, column picker, time cells and
// the first-run split. No backend: every API call is mocked.

const COUNTS = {
  bytes_inline: 0, bytes_packs: 0, committed: 5, error: 0, exchanges: 5,
  history: 5, intruder: 0, markers: 0, repeater: 0,
};

const OLD = 1_700_000_000_000; // 2023-11-14: an older row shows "MMM D"
const NOW = Date.now();

interface Dto {
  id: string; created_at: number; method: string; host: string; path: string;
  status: number; resp_len: number; state: string; tags: string[];
}

const dto = (id: string, host: string, path: string, created_at: number, state = 'committed', tags: string[] = []): Dto => ({
  id, created_at, method: 'GET', host, path, status: 200, resp_len: 5, state, tags,
});

// Newest first (the default sort), so the table order is A..E.
const ROWS: Dto[] = [
  dto('01HZZZZZZZZZZZZZZZZZZZZNAVA', 'shop.example', '/cart', NOW - 1_000, 'committed', ['cart']),
  dto('01HZZZZZZZZZZZZZZZZZZZZNAVB', 'api.other', '/v1/ping', NOW - 2_000),
  dto('01HZZZZZZZZZZZZZZZZZZZZNAVC', 'shop.example', '/checkout', NOW - 3_000),
  dto('01HZZZZZZZZZZZZZZZZZZZZNAVD', 'cdn.static', '/app.js', NOW - 4_000, 'committed', ['shop-asset']),
  dto('01HZZZZZZZZZZZZZZZZZZZZNAVE', 'old.example', '/legacy', OLD),
];
const ID = ROWS.map((r) => r.id);

const b64 = (s: string) => Buffer.from(s, 'latin1').toString('base64');

function exchange(id: string) {
  const r = ROWS.find((x) => x.id === id) ?? ROWS[0];
  const msg = (head: string) => ({
    kind: 'http1-raw', fidelity: 'raw',
    head: { type: 'http1-raw', bytes: b64(head), parser_version: 1 },
    diagnostics: [], body: { type: 'inline', truncated_at: null },
  });
  return {
    id,
    request: msg(`GET ${r.path} HTTP/1.1\r\nHost: ${r.host}\r\n\r\n`),
    response: msg('HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\n'),
    scope_state: 'in', state: r.state, tags: r.tags, notes: '',
  };
}

interface Harness {
  patches: { id: string; body: unknown }[];
  pageErrors: string[];
}

async function mockApi(page: Page, rows: Dto[] = ROWS): Promise<Harness> {
  const h: Harness = { patches: [], pageErrors: [] };
  page.on('pageerror', (e) => h.pageErrors.push(e.message));
  await page.route('**/api/v1/counts', (r) => r.fulfill({ status: 200, json: COUNTS }));
  await page.route(/\/api\/v1\/exchanges\/([^/?]+)$/, (r) => {
    const id = decodeURIComponent(new URL(r.request().url()).pathname.split('/').pop()!);
    if (r.request().method() === 'PATCH') {
      h.patches.push({ id, body: r.request().postDataJSON() });
      return r.fulfill({ status: 200, json: {} });
    }
    return r.fulfill({ status: 200, json: exchange(id) });
  });
  await page.route(/\/api\/v1\/exchanges\/[^/]+\/body\/(req|resp)$/, (r) =>
    r.fulfill({
      status: 200,
      contentType: 'application/octet-stream',
      headers: { 'x-crosswire-length': '5' },
      body: Buffer.from('hello'),
    }),
  );
  await page.route(/\/api\/v1\/exchanges(\?.*)?$/, (r) =>
    r.fulfill({ status: 200, json: { items: rows, next_cursor: null } }),
  );
  return h;
}

const rowsLoc = (page: Page) => page.getByTestId('history-row');
const rowById = (page: Page, id: string) => page.locator(`tr[data-testid="history-row"][data-id="${id}"]`);
const focusedRowId = (page: Page) =>
  page.evaluate(() => (document.activeElement as HTMLElement | null)?.closest('tr[data-testid="history-row"]')?.getAttribute('data-id') ?? null);
const rovingId = (page: Page) =>
  page.locator('tr[data-testid="history-row"][tabindex="0"]').getAttribute('data-id');

async function open(page: Page) {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/');
  await expect(rowsLoc(page)).toHaveCount(ROWS.length);
}

test('/ focuses search; "j" typed in a field never moves row focus', async ({ page }) => {
  const h = await mockApi(page);
  await open(page);
  // Open the second row: the detail (tag input, CodeMirror) is now on screen.
  await rowById(page, ID[1]).locator('td.open').first().click();
  await expect(page.getByTestId('detail-id')).toHaveText(ID[1]);
  await rowById(page, ID[1]).focus();
  expect(await rovingId(page)).toBe(ID[1]);

  // `/` from a row lands in the search box and is not typed into it.
  await page.keyboard.press('/');
  const search = page.getByTestId('history-search');
  await expect(search).toBeFocused();
  await expect(search).toHaveValue('');

  // "j" in the search box is text: it filters, it does not move focus.
  await search.press('j');
  await expect(search).toBeFocused();
  await expect(search).toHaveValue('j');
  await search.fill('');

  // The filter bar, the detail's tag input and its CodeMirror editor.
  for (const field of [
    page.getByTestId('filter-host'),
    page.getByTestId('tag-input'),
    page.getByTestId('exchange-detail').locator('.cm-content').first(),
  ]) {
    await field.click();
    await page.keyboard.press('j');
    await page.keyboard.press('k');
    await page.keyboard.press('ArrowDown');
    expect(await focusedRowId(page)).toBeNull();
    expect(await rovingId(page)).toBe(ID[1]);
  }
  expect(h.pageErrors).toEqual([]);
});

test('j/k and arrows move row focus without wrapping; Tab reaches one row; Enter opens; Space toggles', async ({ page }) => {
  await mockApi(page);
  await open(page);

  // Tab walk from the select-all box: exactly one row is a tab stop.
  await page.getByTestId('select-all').focus();
  const stops: string[] = [];
  for (let i = 0; i < ROWS.length * 2 + 4; i++) {
    await page.keyboard.press('Tab');
    const id = await page.evaluate(() => {
      const el = document.activeElement as HTMLElement | null;
      return el?.matches('tr[data-testid="history-row"]') ? el.getAttribute('data-id') : null;
    });
    if (id) stops.push(id);
  }
  expect(stops).toEqual([ID[0]]);

  await rowById(page, ID[0]).focus();
  // k at the top stays (no wrap to the bottom).
  await page.keyboard.press('k');
  expect(await focusedRowId(page)).toBe(ID[0]);
  await page.keyboard.press('ArrowUp');
  expect(await focusedRowId(page)).toBe(ID[0]);
  await page.keyboard.press('j');
  expect(await focusedRowId(page)).toBe(ID[1]);
  await page.keyboard.press('ArrowDown');
  expect(await focusedRowId(page)).toBe(ID[2]);
  await page.keyboard.press('k');
  expect(await focusedRowId(page)).toBe(ID[1]);
  for (let i = 0; i < ROWS.length + 2; i++) await page.keyboard.press('j');
  // j at the bottom stays (no wrap to the top).
  expect(await focusedRowId(page)).toBe(ID[4]);
  expect(await rovingId(page)).toBe(ID[4]);

  // Space toggles the checkbox and opens nothing.
  await page.keyboard.press(' ');
  await expect(rowById(page, ID[4]).locator('input[type="checkbox"]')).toBeChecked();
  await expect(page.getByTestId('exchange-detail')).toHaveCount(0);
  await expect(page.getByTestId('history-count')).toContainText('1 selected');
  await page.keyboard.press(' ');
  await expect(rowById(page, ID[4]).locator('input[type="checkbox"]')).not.toBeChecked();

  // Enter opens the focused row.
  await page.keyboard.press('k');
  await page.keyboard.press('Enter');
  await expect(page.getByTestId('detail-id')).toHaveText(ID[3]);
  await expect(rowById(page, ID[3])).toHaveClass(/selected/);
});

test('search narrows rows, counts matches, Esc clears to a row; the open row stays open', async ({ page }) => {
  await mockApi(page);
  await open(page);
  await rowById(page, ID[1]).locator('td.open').first().click();
  await expect(page.getByTestId('detail-id')).toHaveText(ID[1]);

  const search = page.getByTestId('history-search');
  await search.fill('shop');
  // Host matches (A, C) and a tag match (D's "shop-asset").
  await expect(rowsLoc(page)).toHaveCount(3);
  await expect(page.getByTestId('history-count')).toContainText(`3 of ${ROWS.length} loaded match`);
  // The open row (B, api.other) is filtered out of the list but stays open.
  await expect(rowById(page, ID[1])).toHaveCount(0);
  await expect(page.getByTestId('detail-id')).toHaveText(ID[1]);

  await search.fill('zzz-nothing');
  await expect(rowsLoc(page)).toHaveCount(0);
  await expect(page.getByTestId('history-empty')).toHaveText('No loaded exchange matches the search.');

  await search.press('Escape');
  await expect(search).toHaveValue('');
  await expect(rowsLoc(page)).toHaveCount(ROWS.length);
  await expect.poll(() => focusedRowId(page)).not.toBeNull();
  await expect(page.getByTestId('history-count')).toContainText(`${ROWS.length} loaded`);
});

test('select-all while searching selects only the visible rows; bulk tag patches only them', async ({ page }) => {
  const h = await mockApi(page);
  await open(page);
  await page.getByTestId('history-search').fill('shop.example');
  await expect(rowsLoc(page)).toHaveCount(2);
  await page.getByTestId('select-all').check();
  await expect(page.getByTestId('history-count')).toContainText('2 of 5 loaded match · 2 selected');
  await page.getByTestId('bulk-tag-input').fill('reviewed');
  await page.getByTestId('bulk-tag-apply').click();
  await expect(page.locator('[data-testid="toast"][data-kind="success"]')).toContainText('Tagged 2 exchanges');
  expect(h.patches.map((p) => p.id).sort()).toEqual([ID[0], ID[2]].sort());
  for (const p of h.patches) expect(JSON.stringify(p.body)).toContain('reviewed');
});

test('column picker hides Tags and Time and the choice survives a reload', async ({ page }) => {
  const rows = [...ROWS.slice(0, 4), dto(ID[4], 'old.example', '/legacy', OLD, 'received')];
  await mockApi(page, rows);
  await open(page);
  const head = page.locator('[data-testid="history-list"] thead');
  await expect(head.locator('th', { hasText: 'Tags' })).toHaveCount(1);
  await expect(head.locator('th', { hasText: 'Time' })).toHaveCount(1);
  // With Time shown, the uncommitted marker sits in the time cell.
  await expect(rowById(page, ID[4]).locator('td.time [data-testid="uncommitted-marker"]')).toHaveCount(1);

  for (const col of ['tags', 'time']) {
    await page.getByTestId('column-picker').click();
    await page.getByTestId('column-menu').getByTestId(`column-${col}`).click();
  }
  const check = async () => {
    await expect(head.locator('th', { hasText: 'Tags' })).toHaveCount(0);
    await expect(head.locator('th', { hasText: 'Time' })).toHaveCount(0);
    await expect(page.locator('[data-testid="history-list"] td.tags')).toHaveCount(0);
    await expect(page.locator('[data-testid="history-list"] td.time')).toHaveCount(0);
    await expect(page.locator('[data-testid="history-list"] col.c-tags, [data-testid="history-list"] col.c-time')).toHaveCount(0);
    // Time hidden: the uncommitted marker moves into the host cell.
    await expect(rowById(page, ID[4]).locator('td.host [data-testid="uncommitted-marker"]')).toHaveCount(1);
    await expect(head.locator('th', { hasText: 'Host' })).toHaveCount(1);
    await expect(head.locator('th', { hasText: 'Path' })).toHaveCount(1);
  };
  await check();
  await page.reload();
  await expect(rowsLoc(page)).toHaveCount(rows.length);
  await check();
});

test('an older row shows "MMM D"; the first-run split is 55/45 on a laptop', async ({ page }) => {
  await mockApi(page);
  await open(page);
  expect(await page.evaluate(() => localStorage.getItem('crosswire.split.history-detail'))).toBeNull();
  await expect(rowById(page, ID[4]).locator('td.time')).toHaveText(/^\s*[A-Z][a-z]{2} \d{1,2} \d{2}:\d{2}/);
  await expect(rowById(page, ID[0]).locator('td.time')).toHaveText(/^\s*\d{2}:\d{2}/);
  const share = async () => {
    const list = await page.getByTestId('history-list').boundingBox();
    const detail = await page.getByTestId('history-detail').boundingBox();
    return list!.width / (list!.width + detail!.width);
  };
  // 1440 px is under the 1600 px wide-window threshold: the list gets ~55%.
  expect(await share()).toBeGreaterThan(0.52);
  expect(await share()).toBeLessThan(0.58);
  // A first run in a wide window gives the list ~45%.
  await page.setViewportSize({ width: 1700, height: 900 });
  await page.reload();
  await expect(rowsLoc(page)).toHaveCount(ROWS.length);
  expect(await page.evaluate(() => localStorage.getItem('crosswire.split.history-detail'))).toBeNull();
  expect(await share()).toBeGreaterThan(0.42);
  expect(await share()).toBeLessThan(0.48);
});

test('a regex-like search query is literal text and raises no error', async ({ page }) => {
  const h = await mockApi(page, [...ROWS, dto('01HZZZZZZZZZZZZZZZZZZZZNAVF', 'x.example', '/a(b[c', NOW - 5_000)]);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/');
  await expect(rowsLoc(page)).toHaveCount(ROWS.length + 1);
  const search = page.getByTestId('history-search');
  for (const q of ['(', '[', '(b[', '\\', '*', '.*', '?', '+', ')', ']']) {
    await search.fill(q);
    const want = '/a(b[c'.includes(q) ? 1 : 0;
    await expect(rowsLoc(page), q).toHaveCount(want);
  }
  expect(h.pageErrors).toEqual([]);
});

test('Load more is enabled while the server has more pages and appends the next page', async ({ page }) => {
  await page.route('**/api/v1/counts', (r) => r.fulfill({ status: 200, json: COUNTS }));
  const afters: (string | null)[] = [];
  await page.route(/\/api\/v1\/exchanges(\?.*)?$/, (r) => {
    const after = new URL(r.request().url()).searchParams.get('after');
    afters.push(after);
    const items = after === null ? ROWS.slice(0, 3) : ROWS.slice(3);
    return r.fulfill({ status: 200, json: { items, next_cursor: after === null ? 'page-2' : null } });
  });
  await page.goto('/');
  await expect(rowsLoc(page)).toHaveCount(3);
  const more = page.getByTestId('load-more');
  await expect(more).toHaveText('Load more');
  await expect(more).toBeEnabled();
  await more.click();
  await expect(rowsLoc(page)).toHaveCount(ROWS.length);
  expect(afters).toEqual([null, 'page-2']);
  await expect(more).toHaveText('End');
  await expect(more).toBeDisabled();
});
