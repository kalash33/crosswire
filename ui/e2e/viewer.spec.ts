import { test, expect, type Page } from '@playwright/test';

// U2 D1: the message viewer in the History detail (Pretty | Raw | Headers |
// Hex, colouring, wrap, control characters, find, large bodies).

const COUNTS = {
  bytes_inline: 0, bytes_packs: 0, committed: 0, error: 0, exchanges: 1,
  history: 1, intruder: 0, markers: 0, repeater: 0,
};
const ID = '01HZZZZZZZZZZZZZZZZZZZVIEW1';
const b64 = (s: string) => Buffer.from(s, 'latin1').toString('base64');

interface Msg {
  head: string;
  body: Buffer | null;
}

async function mockExchange(page: Page, req: Msg, resp: Msg) {
  await page.route('**/api/v1/counts', (r) => r.fulfill({ status: 200, json: COUNTS }));
  const message = (m: Msg) => ({
    kind: 'http1-raw',
    fidelity: 'raw',
    head: { type: 'http1-raw', bytes: b64(m.head), parser_version: 1 },
    diagnostics: [],
    body: m.body ? { type: 'inline', truncated_at: null } : { type: 'none' },
  });
  await page.route(/\/api\/v1\/exchanges\/[^/?]+$/, (r) =>
    r.fulfill({
      status: 200,
      json: { id: ID, request: message(req), response: message(resp), scope_state: 'in', state: 'committed', tags: [], notes: '' },
    }),
  );
  const body = (m: Msg) => (r: import('@playwright/test').Route) =>
    m.body
      ? r.fulfill({
          status: 200,
          contentType: 'application/octet-stream',
          headers: { 'x-crosswire-length': String(m.body.length) },
          body: m.body,
        })
      : r.fulfill({ status: 404, json: { error: { code: 'not_found', message: 'no body' } } });
  await page.route(/\/api\/v1\/exchanges\/[^/]+\/body\/req$/, body(req));
  await page.route(/\/api\/v1\/exchanges\/[^/]+\/body\/resp$/, body(resp));
  await page.route(/\/api\/v1\/exchanges(\?.*)?$/, (r) =>
    r.fulfill({
      status: 200,
      json: {
        items: [{ id: ID, created_at: 1_700_000_000_000, method: 'POST', host: 'api.example', path: '/v1/items?page=2&q=a+b', status: 200, resp_len: resp.body?.length ?? 0, state: 'committed', tags: [] }],
        next_cursor: null,
      },
    }),
  );
}

async function openDetail(page: Page) {
  await page.goto('/');
  await page.getByTestId('history-row').locator('td.open').first().click();
  // The exchange has loaded once its id shows (the sides render with it).
  await expect(page.getByTestId('detail-id')).toBeVisible();
}

// Narrow panes show one message behind a switch; side by side it is a no-op.
async function pane(page: Page, side: 'req' | 'resp') {
  const sw = page.getByTestId(side === 'req' ? 'detail-side-req' : 'detail-side-resp');
  if ((await sw.count()) > 0) await sw.click();
  return page.getByTestId(side === 'req' ? 'detail-request' : 'detail-response');
}

const JSON_BODY = '{"user":{"id":7,"name":"ann","admin":false},"tags":["a","b"],"big":12345678901234567890}';
const FORM_BODY = 'user=ann&note=two+words&html=%3Cb%3Ehi%3C%2Fb%3E';

test.beforeEach(async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
});

test('a JSON response opens on Pretty with formatted, coloured text', async ({ page }) => {
  await mockExchange(
    page,
    { head: 'POST /v1/items?page=2&q=a+b HTTP/1.1\r\nHost: api.example\r\nContent-Type: application/x-www-form-urlencoded\r\n\r\n', body: Buffer.from(FORM_BODY) },
    { head: 'HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n', body: Buffer.from(JSON_BODY) },
  );
  await openDetail(page);
  const resp = await pane(page, 'resp');
  await expect(resp.getByTestId('detail-tab-pretty')).toHaveAttribute('aria-selected', 'true');
  const pretty = page.getByTestId('detail-resp-pretty-json');
  await expect(pretty.locator('.cm-content')).toContainText('"name": "ann"');
  // Recorded digits survive (no JSON.parse/stringify round trip).
  await expect(pretty.locator('.cm-content')).toContainText('12345678901234567890');
  expect(await pretty.locator('.cm-line').count()).toBeGreaterThan(5);
  await expect(pretty.locator('.tok-propertyName').first()).toHaveText('"user"');
  await expect(pretty.locator('.tok-number').first()).toHaveText('7');
  await expect(pretty.locator('.tok-keyword').first()).toHaveText('false');

  // Raw colours the head as HTTP and the body as JSON.
  await resp.getByTestId('detail-tab-raw').click();
  await expect(page.getByTestId('detail-resp-head').locator('.tok-statusLine')).toContainText('HTTP/1.1 200 OK');
  await expect(page.getByTestId('detail-resp-head').locator('.tok-headerName').first()).toHaveText('Content-Type');
  await expect(page.getByTestId('detail-resp-body').locator('.tok-propertyName').first()).toHaveText('"user"');
});

test('a form request shows the query and form tables', async ({ page }) => {
  await mockExchange(
    page,
    { head: 'POST /v1/items?page=2&q=a+b HTTP/1.1\r\nHost: api.example\r\nContent-Type: application/x-www-form-urlencoded\r\n\r\n', body: Buffer.from(FORM_BODY) },
    { head: 'HTTP/1.1 204 No Content\r\n\r\n', body: null },
  );
  await openDetail(page);
  const req = await pane(page, 'req');
  await expect(req.getByTestId('detail-tab-pretty')).toHaveAttribute('aria-selected', 'true');
  const query = page.getByTestId('detail-req-pretty-query');
  await expect(query.getByTestId('detail-req-pretty-query-key')).toHaveText(['page', 'q']);
  await expect(query.getByTestId('detail-req-pretty-query-value')).toHaveText(['2', 'a b']);
  const form = page.getByTestId('detail-req-pretty-form');
  await expect(form.getByTestId('detail-req-pretty-form-key')).toHaveText(['user', 'note', 'html']);
  await expect(form.getByTestId('detail-req-pretty-form-value')).toHaveText(['ann', 'two words', '<b>hi</b>']);
  await expect(form.getByTestId('detail-req-pretty-form-raw')).toHaveText(FORM_BODY);
  await expect(form.locator('b')).toHaveCount(0);
  // The response has no body and no JSON: it opens on Raw.
  const resp = await pane(page, 'resp');
  await expect(resp.getByTestId('detail-tab-raw')).toHaveAttribute('aria-selected', 'true');
});

test('valid JSON served as text/html keeps Raw as default but offers Pretty', async ({ page }) => {
  await mockExchange(
    page,
    { head: 'GET /x HTTP/1.1\r\nHost: api.example\r\n\r\n', body: null },
    { head: 'HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n', body: Buffer.from(JSON_BODY) },
  );
  await openDetail(page);
  const resp = await pane(page, 'resp');
  await expect(resp.getByTestId('detail-tab-raw')).toHaveAttribute('aria-selected', 'true');
  await expect(resp.getByTestId('detail-tab-pretty')).toContainText('JSON');
  await resp.getByTestId('detail-tab-pretty').click();
  await expect(page.getByTestId('detail-resp-pretty-json').locator('.cm-content')).toContainText('"id": 7');
});

test('a 5 MB JSON body: Pretty refuses with a note, Raw shows 1 MiB with Show all', async ({ page }) => {
  const items = Array.from({ length: 170_000 }, (_, i) => `{"i":${i},"v":"xxxxxxxxxxxx"}`).join(',');
  const big = Buffer.from(`[${items}]`);
  expect(big.length).toBeGreaterThan(4 * 1024 * 1024);
  await mockExchange(
    page,
    { head: 'GET /big HTTP/1.1\r\nHost: api.example\r\n\r\n', body: null },
    { head: 'HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n', body: big },
  );
  await openDetail(page);
  const resp = await pane(page, 'resp');
  await expect(resp.getByTestId('detail-tab-raw')).toHaveAttribute('aria-selected', 'true');
  await expect(resp.getByTestId('detail-clipped')).toContainText('first 1.0 MB');
  await resp.getByTestId('detail-tab-pretty').click();
  await expect(page.getByTestId('detail-resp-pretty-note')).toContainText('over the 2.0 MB limit');
  await resp.getByTestId('detail-tab-raw').click();
  const t0 = Date.now();
  await resp.getByTestId('detail-show-all').click();
  await expect(resp.getByTestId('detail-clipped')).toHaveCount(0);
  await expect(page.getByTestId('detail-resp-body').locator('.cm-content')).toContainText('{"i":0,');
  // The page stays responsive: a click round-trips quickly.
  await resp.getByTestId('detail-tab-hex').click();
  await expect(page.getByTestId('detail-resp-hex')).toBeVisible();
  expect(Date.now() - t0).toBeLessThan(10_000);
});

test('wrap and control-character toggles change the editor classes', async ({ page }) => {
  await mockExchange(
    page,
    { head: 'GET /x HTTP/1.1\r\nHost: api.example\r\n\r\n', body: null },
    { head: 'HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\n', body: Buffer.from('line one\r\nline two\r\n') },
  );
  await openDetail(page);
  const resp = await pane(page, 'resp');
  const body = page.getByTestId('detail-resp-body');
  await expect(body.locator('.cm-content')).toHaveClass(/cm-lineWrapping/);
  await expect(resp.getByTestId('detail-wrap')).toHaveAttribute('aria-pressed', 'true');
  await resp.getByTestId('detail-wrap').click();
  await expect(body.locator('.cm-content')).not.toHaveClass(/cm-lineWrapping/);

  const editor = body.getByTestId('byte-editor');
  await expect(editor).toHaveClass(/hide-controls/);
  await expect(body.locator('.cm-ctl').first()).toBeHidden();
  await resp.getByTestId('detail-controls').click();
  await expect(editor).toHaveClass(/show-controls/);
  await expect(body.locator('.cm-ctl[data-byte="13"]').first()).toBeVisible();
  await expect(body.locator('.cm-ctl[data-byte="10"]').first()).toBeVisible();
});

test('find counts matches, steps with wrap-around and Esc closes', async ({ page }) => {
  await mockExchange(
    page,
    { head: 'GET /x HTTP/1.1\r\nHost: api.example\r\n\r\n', body: null },
    { head: 'HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nX-Token: abc\r\n\r\n', body: Buffer.from('token=ABC; Token=abc; none') },
  );
  await openDetail(page);
  const resp = await pane(page, 'resp');
  await resp.getByTestId('detail-find').click();
  const input = resp.getByTestId('detail-findbar-input');
  await expect(input).toBeFocused();
  await input.fill('token');
  // Head "X-Token" + body "token" and "Token" (case-insensitive by default).
  await expect(resp.getByTestId('detail-findbar-count')).toHaveText('1/3');
  await expect(resp.locator('.cm-find-match')).toHaveCount(3);
  await expect(resp.locator('.cm-find-current')).toHaveCount(1);
  await input.press('Enter');
  await expect(resp.getByTestId('detail-findbar-count')).toHaveText('2/3');
  await input.press('Enter');
  await input.press('Enter');
  await expect(resp.getByTestId('detail-findbar-count')).toHaveText('1/3');
  await input.press('Shift+Enter');
  await expect(resp.getByTestId('detail-findbar-count')).toHaveText('3/3');
  await resp.getByTestId('detail-findbar-case').click();
  await expect(resp.getByTestId('detail-findbar-count')).toHaveText('1/1');
  await input.fill('zzz');
  await expect(resp.getByTestId('detail-findbar-count')).toHaveText('No matches');
  await input.press('Escape');
  await expect(resp.getByTestId('detail-findbar')).toHaveCount(0);
});

test('Ctrl+F opens find inside the viewer and is not swallowed outside it', async ({ page }) => {
  await mockExchange(
    page,
    { head: 'GET /x HTTP/1.1\r\nHost: api.example\r\n\r\n', body: null },
    { head: 'HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\n', body: Buffer.from('hello') },
  );
  await openDetail(page);
  await page.evaluate(() => {
    (window as unknown as { __prevented: boolean[] }).__prevented = [];
    window.addEventListener('keydown', (e) => {
      if (e.key.toLowerCase() === 'f' && e.ctrlKey) {
        (window as unknown as { __prevented: boolean[] }).__prevented.push(e.defaultPrevented);
      }
    });
  });
  // Outside: the History host filter field.
  await page.getByTestId('filter-host').focus();
  await page.keyboard.press('Control+f');
  expect(await page.evaluate(() => (window as unknown as { __prevented: boolean[] }).__prevented)).toEqual([false]);
  await expect(page.getByTestId('detail-findbar')).toHaveCount(0);

  // Inside: focus the response viewer, Ctrl+F opens its find bar.
  const resp = await pane(page, 'resp');
  await resp.getByTestId('detail-viewer').focus();
  await page.keyboard.press('Control+f');
  await expect(resp.getByTestId('detail-findbar-input')).toBeFocused();
});
