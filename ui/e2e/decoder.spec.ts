import { test, expect, type Page } from '@playwright/test';

const ID = '01HZZZZZZZZZZZZZZZZZZZZDECO';
const BODY = 'SGVsbG8gd29ybGQ=';
const b64 = (s: string) => Buffer.from(s, 'latin1').toString('base64');
const msg = (head: string, body: unknown) => ({
  kind: 'http1-raw', fidelity: 'raw', diagnostics: [],
  head: { type: 'http1-raw', bytes: b64(head), parser_version: 1 }, body,
});

async function mock(page: Page) {
  await page.route('**/api/v1/counts', (r) => r.fulfill({ status: 200, json: { exchanges: 1 } }));
  await page.route('**/api/v1/scope', (r) => r.fulfill({ status: 200, json: { include: [], exclude: [] } }));
  await page.route(/\/api\/v1\/intercept$/, (r) =>
    r.fulfill({ status: 200, json: { enabled: false, rules: { skip_extensions: [], skip_out_of_scope: false, overflow: 'forward-newest', on_timeout: 'forward' }, held: [] } }),
  );
  await page.route(/\/api\/v1\/exchanges\/[^/]+\/body\/req$/, (r) =>
    r.fulfill({ status: 404, json: { error: { code: 'not_found', message: 'x' } } }),
  );
  await page.route(/\/api\/v1\/exchanges\/[^/]+\/body\/resp$/, (r) =>
    r.fulfill({ status: 200, contentType: 'application/octet-stream', headers: { 'x-crosswire-length': String(BODY.length) }, body: Buffer.from(BODY, 'latin1') }),
  );
  await page.route(/\/api\/v1\/exchanges\/[^/?]+$/, (r) =>
    r.fulfill({
      status: 200,
      json: {
        id: ID,
        request: msg('GET / HTTP/1.1\r\nHost: d.example\r\n\r\n', { type: 'none' }),
        response: msg('HTTP/1.1 200 OK\r\n\r\n', { type: 'inline', truncated_at: BODY.length }),
        scope_state: 'in', state: 'committed', tags: [], notes: '',
      },
    }),
  );
  await page.route(/\/api\/v1\/exchanges(\?.*)?$/, (r) =>
    r.fulfill({ status: 200, json: { items: [{ id: ID, created_at: 1_700_000_000_000, method: 'GET', host: 'd.example', path: '/', status: 200, resp_len: BODY.length, state: 'committed', tags: [] }], next_cursor: null } }),
  );
}

async function selectBody(page: Page) {
  await page.getByTestId('history-row').first().click();
  const body = page.getByTestId('detail-resp-body');
  await expect(body).toContainText(BODY);
  await body.evaluate((el) => {
    const range = document.createRange();
    range.selectNodeContents(el.querySelector('.cm-content') ?? el);
    const sel = window.getSelection()!;
    sel.removeAllRanges();
    sel.addRange(range);
  });
}

async function openPane(page: Page) {
  await page.keyboard.press('Control+Shift+D');
  await expect(page.getByTestId('decoder-pane')).toBeVisible();
}

async function openSeeded(page: Page) {
  await mock(page);
  await page.goto('/');
  await selectBody(page);
  await openPane(page);
}

async function addStep(page: Page, i: number, codec: string, dir: string) {
  await page.getByTestId('decoder-step-add').click();
  await page.getByTestId(`decoder-step-${i}-codec`).selectOption(codec);
  await page.getByTestId(`decoder-step-${i}-dir`).selectOption(dir);
}

test('chord_opens_pane_seeded_from_selection', async ({ page }) => {
  await openSeeded(page);
  await expect(page.getByTestId('decoder-pane')).toHaveAttribute('role', 'dialog');
  await expect(page.getByTestId('decoder-input')).toHaveValue(BODY);
});

test('base64_decode_step_outputs_text', async ({ page }) => {
  await openSeeded(page);
  await addStep(page, 0, 'base64', 'decode');
  await expect(page.getByTestId('decoder-output-0')).toHaveText('Hello world');
});

test('chained_steps', async ({ page }) => {
  await openSeeded(page);
  await addStep(page, 0, 'base64', 'decode');
  await addStep(page, 1, 'url', 'encode');
  await expect(page.getByTestId('decoder-output-1')).toHaveText('Hello%20world');
});

test('warning_shown_on_malformed', async ({ page }) => {
  await openSeeded(page);
  await page.getByTestId('decoder-input').fill('abc');
  await addStep(page, 0, 'hex', 'decode');
  await expect(page.getByTestId('decoder-warning-0')).toHaveText('hex: odd length, last nibble dropped');
});

test('jwt_view_appears_with_banner', async ({ page }) => {
  await openSeeded(page);
  const u = (s: string) => Buffer.from(s).toString('base64url');
  await page.getByTestId('decoder-input').fill(`${u('{"alg":"HS256"}')}.${u('{"sub":"1"}')}.c2ln`);
  await expect(page.getByTestId('decoder-jwt')).toBeVisible();
  await expect(page.getByTestId('decoder-jwt-banner')).toHaveText('signature not verified');
});

test('copy_writes_clipboard', async ({ page, context }) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write']);
  await openSeeded(page);
  await addStep(page, 0, 'base64', 'decode');
  await page.getByTestId('decoder-copy-0').click();
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe('Hello world');
});

test('escape_closes_pane', async ({ page }) => {
  await openSeeded(page);
  await page.keyboard.press('Escape');
  await expect(page.getByTestId('decoder-pane')).toHaveCount(0);
});

test('empty_selection_opens_empty_pane', async ({ page }) => {
  await mock(page);
  await page.goto('/');
  await expect(page.getByTestId('app-shell')).toBeVisible();
  await openPane(page);
  await expect(page.getByTestId('decoder-input')).toHaveValue('');
});
