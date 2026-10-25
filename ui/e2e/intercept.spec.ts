import { test, expect, type Page } from '@playwright/test';

// No backend: `vite preview` serves the bundle; every API call is mocked.

const COUNTS = {
  bytes_inline: 0, bytes_packs: 0, committed: 0, error: 0, exchanges: 0,
  history: 0, intruder: 0, markers: 0, repeater: 0,
};
const RULES = {
  skip_extensions: ['png'],
  skip_out_of_scope: false,
  overflow: 'forward-newest',
  on_timeout: 'forward',
};
const ID_A = '01HZZZZZZZZZZZZZZZZZZZZZAA';
const ID_B = '01HZZZZZZZZZZZZZZZZZZZZZBB';
const HEAD = 'POST /a HTTP/1.1\r\nHost: a.test\r\nContent-Length: 3\r\n\r\n';
const BODY = 'abc';

interface Harness {
  forwards: { id: string; body: string | null }[];
  rulesPuts: unknown[];
  enabledPuts: unknown[];
}

async function mockApi(page: Page, held: unknown[]): Promise<Harness> {
  const h: Harness = { forwards: [], rulesPuts: [], enabledPuts: [] };
  let enabled = false;
  await page.route('**/api/v1/counts', (r) => r.fulfill({ status: 200, json: COUNTS }));
  await page.route('**/api/v1/scope', (r) => r.fulfill({ status: 200, json: { include: [], exclude: [] } }));
  await page.route(/\/api\/v1\/repeater\/tabs/, (r) => r.fulfill({ status: 200, json: [] }));
  await page.route(/\/api\/v1\/intercept$/, (r) =>
    r.fulfill({ status: 200, json: { enabled, rules: RULES, held: enabled ? held : [] } }),
  );
  await page.route(/\/api\/v1\/intercept\/enabled$/, (r) => {
    const b = r.request().postDataJSON() as { enabled: boolean };
    h.enabledPuts.push(b);
    enabled = b.enabled;
    r.fulfill({ status: 200, body: '' });
  });
  await page.route(/\/api\/v1\/intercept\/rules$/, (r) => {
    h.rulesPuts.push(r.request().postDataJSON());
    r.fulfill({ status: 200, body: '' });
  });
  await page.route(/\/api\/v1\/intercept\/held\/[^/]+\/forward$/, (r) => {
    const id = r.request().url().split('/').slice(-2)[0];
    h.forwards.push({ id, body: r.request().postData() });
    r.fulfill({ status: 200, body: '' });
  });
  await page.route(/\/api\/v1\/intercept\/held\/[^/]+\/head$/, (r) =>
    r.fulfill({ status: 200, contentType: 'application/octet-stream', body: Buffer.from(HEAD, 'latin1') }),
  );
  await page.route(/\/api\/v1\/intercept\/held\/[^/]+\/body$/, (r) => {
    if (r.request().url().includes(ID_B)) {
      return r.fulfill({ status: 404, json: { error: { code: 'not_found', message: 'no body' } } });
    }
    return r.fulfill({ status: 200, contentType: 'application/octet-stream', body: Buffer.from(BODY, 'latin1') });
  });
  return h;
}

const A = { id: ID_A, host: 'a.test', method: 'POST', path: '/a', size: 60, editable: true };
const B = { id: ID_B, host: 'b.test', method: 'POST', path: '/b', size: 9999999, editable: false };

async function openIntercept(page: Page) {
  await page.goto('/');
  await expect(page.getByTestId('app-shell')).toBeVisible();
  await page.getByTestId('nav-intercept').click();
  await expect(page.getByTestId('intercept-view')).toBeVisible();
}

test('toggle on from the status bar shows the held list; edit + forward posts edited head', async ({ page }) => {
  const h = await mockApi(page, [A]);
  await openIntercept(page);
  await expect(page.getByTestId('status-intercept')).toContainText('OFF');
  await expect(page.getByTestId('held-row')).toHaveCount(0);

  await page.getByTestId('intercept-toggle').click();
  await expect.poll(() => h.enabledPuts).toEqual([{ enabled: true }]);
  // Hold arrives via an event-driven refresh in production; here re-GET via reload of view.
  await page.getByTestId('nav-history').click();
  await page.getByTestId('nav-intercept').click();
  await expect(page.getByTestId('held-row')).toHaveCount(1);
  await expect(page.getByTestId('held-row')).toContainText('/a');

  const content = page.locator('.cm-content').first();
  await expect(content).toContainText('POST /a HTTP/1.1');
  await content.click();
  await page.keyboard.press('ControlOrMeta+ArrowDown'); // doc end (the body)
  await page.keyboard.type('X');

  await page.getByTestId('forward').click();
  await expect.poll(() => h.forwards.length).toBe(1);
  const fb = JSON.parse(h.forwards[0].body!) as { head: string; body?: string };
  const head = Buffer.from(fb.head, 'base64').toString('latin1');
  // An edited body always travels with the (re-sent) head.
  expect(head).toBe(HEAD);
  expect(Buffer.from(fb.body!, 'base64').toString('latin1')).toBe(BODY + 'X');
});

test('unedited forward posts no body (original bytes)', async ({ page }) => {
  const h = await mockApi(page, [A]);
  await openIntercept(page);
  await page.getByTestId('intercept-toggle').click();
  await page.getByTestId('nav-history').click();
  await page.getByTestId('nav-intercept').click();
  await expect(page.getByTestId('held-row')).toHaveCount(1);
  await expect(page.locator('.cm-content').first()).toContainText('POST /a');
  await page.getByTestId('forward').click();
  await expect.poll(() => h.forwards.length).toBe(1);
  expect(h.forwards[0].body === null || h.forwards[0].body === '').toBe(true);
});

test('a non-editable item is read-only with the badge and forwards the original', async ({ page }) => {
  const h = await mockApi(page, [B]);
  await openIntercept(page);
  await page.getByTestId('intercept-toggle').click();
  await page.getByTestId('nav-history').click();
  await page.getByTestId('nav-intercept').click();
  await expect(page.getByTestId('not-editable-badge')).toBeVisible();
  const content = page.locator('.cm-content').first();
  await expect(content).toContainText('POST /a');
  await expect(content).toHaveAttribute('contenteditable', 'false');
  await page.getByTestId('forward').click();
  await expect.poll(() => h.forwards.length).toBe(1);
  expect(h.forwards[0].id).toBe(ID_B);
  expect(h.forwards[0].body === null || h.forwards[0].body === '').toBe(true);
});

test('rules editor round-trips a RulesDto PUT', async ({ page }) => {
  const h = await mockApi(page, []);
  await openIntercept(page);
  await page.getByText('Rules', { exact: true }).click();
  await expect(page.getByTestId('rules-skip-ext')).toHaveValue('png');
  await page.getByTestId('rules-skip-ext').fill('png, .jpg css');
  await page.getByTestId('rules-skip-oos').check();
  await page.getByTestId('rules-overflow').selectOption('forward-oldest');
  await page.getByTestId('rules-timeout').selectOption('respond-504');
  await page.getByTestId('rules-save').click();
  await expect.poll(() => h.rulesPuts.length).toBe(1);
  expect(h.rulesPuts[0]).toEqual({
    skip_extensions: ['png', 'jpg', 'css'],
    skip_out_of_scope: true,
    overflow: 'forward-oldest',
    on_timeout: 'respond-504',
  });
});
