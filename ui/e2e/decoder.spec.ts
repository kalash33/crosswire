import { test, expect, type Page } from '@playwright/test';

const ID = '01HZZZZZZZZZZZZZZZZZZZZDECO';
const BODY = 'SGVsbG8gd29ybGQ=';
const b64 = (s: string) => Buffer.from(s, 'latin1').toString('base64');
const msg = (head: string, body: unknown) => ({
  kind: 'http1-raw', fidelity: 'raw', diagnostics: [],
  head: { type: 'http1-raw', bytes: b64(head), parser_version: 1 }, body,
});

async function mock(page: Page, bodyBytes: Buffer = Buffer.from(BODY, 'latin1')) {
  await page.route('**/api/v1/counts', (r) => r.fulfill({ status: 200, json: { exchanges: 1 } }));
  await page.route('**/api/v1/scope', (r) => r.fulfill({ status: 200, json: { include: [], exclude: [] } }));
  await page.route(/\/api\/v1\/intercept$/, (r) =>
    r.fulfill({ status: 200, json: { enabled: false, rules: { skip_extensions: [], skip_out_of_scope: false, overflow: 'forward-newest', on_timeout: 'forward' }, held: [] } }),
  );
  await page.route(/\/api\/v1\/exchanges\/[^/]+\/body\/req$/, (r) =>
    r.fulfill({ status: 404, json: { error: { code: 'not_found', message: 'x' } } }),
  );
  await page.route(/\/api\/v1\/exchanges\/[^/]+\/body\/resp$/, (r) =>
    r.fulfill({ status: 200, contentType: 'application/octet-stream', headers: { 'x-crosswire-length': String(bodyBytes.length) }, body: bodyBytes }),
  );
  await page.route(/\/api\/v1\/exchanges\/[^/?]+$/, (r) =>
    r.fulfill({
      status: 200,
      json: {
        id: ID,
        request: msg('GET / HTTP/1.1\r\nHost: d.example\r\n\r\n', { type: 'none' }),
        response: msg('HTTP/1.1 200 OK\r\n\r\n', { type: 'inline', truncated_at: bodyBytes.length }),
        scope_state: 'in', state: 'committed', tags: [], notes: '',
      },
    }),
  );
  await page.route(/\/api\/v1\/exchanges(\?.*)?$/, (r) =>
    r.fulfill({ status: 200, json: { items: [{ id: ID, created_at: 1_700_000_000_000, method: 'GET', host: 'd.example', path: '/', status: 200, resp_len: bodyBytes.length, state: 'committed', tags: [] }], next_cursor: null } }),
  );
}

async function selectBody(page: Page, ready: string | null = BODY) {
  await page.getByTestId('history-row').first().click();
  const body = page.getByTestId('detail-resp-body');
  if (ready) await expect(body).toContainText(ready);
  await body.locator('.cm-content').focus();
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
  // U1 D6: the pane is the /decoder route's content (a region), no longer a
  // floating dialog; the chord navigates there.
  await expect(page).toHaveURL(/\/decoder$/);
  await expect(page.getByTestId('decoder-pane')).toHaveAttribute('role', 'region');
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

test('seeded_high_byte_survives_edit', async ({ page }) => {
  await mock(page, Buffer.from([0x41, 0xe9]));
  await page.goto('/');
  await selectBody(page, 'A');
  await openPane(page);
  const input = page.getByTestId('decoder-input');
  await input.click();
  await page.keyboard.press('Control+End');
  await page.keyboard.type(' ');
  await addStep(page, 0, 'hex', 'encode');
  await expect(page.getByTestId('decoder-output-0')).toHaveText('41e920');
  await expect(page.getByTestId('decoder-input-chip')).toHaveCount(0);
});

test('typed_non_latin1_is_utf8_with_chip', async ({ page }) => {
  await openSeeded(page);
  await page.getByTestId('decoder-input').fill('\u20ac');
  await addStep(page, 0, 'hex', 'encode');
  await expect(page.getByTestId('decoder-output-0')).toHaveText('e282ac');
  await expect(page.getByTestId('decoder-input-chip')).toBeVisible();
});

test('selection_over_nonprintables_is_byte_exact', async ({ page }) => {
  await mock(page, Buffer.from([0x41, 0x7f, 0x85, 0x42]));
  await page.goto('/');
  await selectBody(page, null);
  await openPane(page);
  await addStep(page, 0, 'hex', 'encode');
  await expect(page.getByTestId('decoder-output-0')).toHaveText('417f8542');
});

test('second_chord_reseeds_pane', async ({ page }) => {
  await openSeeded(page);
  await expect(page.getByTestId('decoder-input')).toHaveValue(BODY);
  await page.evaluate(() => {
    const el = document.querySelector('.brand')!;
    const range = document.createRange();
    range.selectNodeContents(el);
    const sel = window.getSelection()!;
    sel.removeAllRanges();
    sel.addRange(range);
  });
  await page.keyboard.press('Control+Shift+D');
  await expect(page.getByTestId('decoder-input')).toHaveValue('Crosswire');
});

test('seeded_crlf_survives_edit', async ({ page }) => {
  await mock(page, Buffer.from([0x61, 0x0d, 0x0a, 0x62]));
  await page.goto('/');
  await selectBody(page, null);
  await openPane(page);
  const input = page.getByTestId('decoder-input');
  await expect(input).toHaveValue('a\u240d\nb');
  await input.click();
  await page.keyboard.press('Control+End');
  await page.keyboard.type('c');
  await addStep(page, 0, 'hex', 'encode');
  await expect(page.getByTestId('decoder-output-0')).toHaveText('610d0a6263');
  await expect(page.getByTestId('decoder-input-chip')).toHaveCount(0);
});

test('copy_failure_shows_chip', async ({ page }) => {
  await page.addInitScript(() => {
    Object.defineProperty(navigator, 'clipboard', {
      value: { writeText: () => Promise.reject(new Error('denied')) },
    });
  });
  await openSeeded(page);
  await addStep(page, 0, 'base64', 'decode');
  await page.getByTestId('decoder-copy-0').click();
  await expect(page.getByTestId('decoder-copy-error')).toHaveText('Copy failed: clipboard unavailable.');
});

test('unicode_selection_outside_editor_is_utf8', async ({ page }) => {
  await mock(page);
  await page.route(/\/api\/v1\/exchanges\/[^/?]+$/, (r) =>
    r.fulfill({
      status: 200,
      json: {
        id: ID,
        request: msg('GET / HTTP/1.1\r\nHost: d.example\r\n\r\n', { type: 'none' }),
        response: null,
        scope_state: 'in', state: 'committed', tags: ['日本'], notes: '',
      },
    }),
  );
  await page.goto('/');
  await page.getByTestId('history-row').first().click();
  const chip = page.getByTestId('tag-chip').first();
  await expect(chip).toContainText('日本');
  await chip.evaluate((el) => {
    const range = document.createRange();
    range.selectNodeContents(el.firstChild as Node);
    const sel = window.getSelection()!;
    sel.removeAllRanges();
    sel.addRange(range);
  });
  await openPane(page);
  await addStep(page, 0, 'hex', 'encode');
  await expect(page.getByTestId('decoder-output-0')).toHaveText('e697a5e69cac');
  await expect(page.getByTestId('decoder-input-chip')).toBeVisible();
});

test('escape_inside_a_field_keeps_the_route', async ({ page }) => {
  await openSeeded(page);
  await page.getByTestId('decoder-input').click();
  await page.keyboard.press('Escape');
  await expect(page.getByTestId('decoder-pane')).toBeVisible();
  await expect(page.getByTestId('decoder-input')).toHaveValue(BODY);
  await page.getByTestId('decoder-step-add').click();
  await page.getByTestId('decoder-step-0-codec').focus();
  await page.keyboard.press('Escape');
  await expect(page.getByTestId('decoder-pane')).toBeVisible();
});

test('back_button_returns_to_history', async ({ page }) => {
  await openSeeded(page);
  await page.getByTestId('decoder-close').click();
  await expect(page.getByTestId('decoder-pane')).toHaveCount(0);
  await expect(page.getByTestId('history-view')).toBeVisible();
});
