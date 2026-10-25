import { test, expect, type Page } from '@playwright/test';
import { XSS_CORPUS } from './fixtures/xss-corpus';
import { watch, liveProbe } from './fixtures/xss-detect';

// XSS gate for the repeater and intercept surfaces. Both render wire data:
// repeater response bytes, a loaded draft and the send-result fields; intercept
// held host/method/path and the held head/body bytes. The corpus is inert DATA
// in mocked responses; nothing here executes it. The detector self-test lives
// in xss-corpus.spec.ts and exercises the same listeners (fixtures/xss-detect.ts).

const COUNTS = {
  bytes_inline: 0, bytes_packs: 0, committed: 0, error: 0, exchanges: 0,
  history: 0, intruder: 0, markers: 0, repeater: 0,
};
const RULES = { skip_extensions: [], skip_out_of_scope: false, overflow: 'forward-newest', on_timeout: 'forward' };
const ALL = XSS_CORPUS.map((p) => p.value).join(' ');
const ONE_LINE = ALL.replace(/[\r\n]/g, '');
const LINES = XSS_CORPUS.map((p) => p.value).join('\n');
const EX_ID = '01HZZZZZZZZZZZZZZZZZZZXSSR';
const HELD_ID = '01HZZZZZZZZZZZZZZZZZZZXSSH';

const REQ = `POST /${ONE_LINE} HTTP/1.1\r\nHost: ${ONE_LINE}\r\nX-Evil: ${ONE_LINE}\r\nContent-Length: ${LINES.length}\r\n\r\n${LINES}`;

// Hover/focus-driven handlers would fire on interaction; poke the page too.
async function settle(page: Page) {
  await page.mouse.move(5, 5);
  await page.mouse.move(300, 300);
  await page.keyboard.press('Tab');
  await page.waitForTimeout(500);
}

test('repeater: corpus in a loaded draft, send result and response bytes is inert', async ({ page, baseURL }) => {
  const origin = new URL(baseURL!).origin;
  const violations = watch(page, origin);

  await page.route('**/api/v1/counts', (r) => r.fulfill({ status: 200, json: COUNTS }));
  await page.route(/\/api\/v1\/repeater\/tabs$/, (r) =>
    r.fulfill({
      status: 200,
      json: [{
        tab_id: 'xss-tab',
        updated_at: 1,
        json: { conn: { host: ONE_LINE, port: 80, tls: false, reuse: false, follow: 'off', sni: ONE_LINE }, mode: 'raw', raw: REQ },
      }],
    }),
  );
  await page.route(/\/api\/v1\/repeater\/tabs\/[^/]+$/, (r) => r.fulfill({ status: 200, body: '' }));
  await page.route(/\/api\/v1\/repeater\/send$/, (r) =>
    r.fulfill({
      status: 200,
      json: {
        exchange_id: EX_ID,
        scope_state: ONE_LINE,
        changes: [],
        skipped: XSS_CORPUS.map((p) => p.value),
        cert_override: false,
        ttfb_ms: 1,
        total_ms: 2,
        redirects: XSS_CORPUS.map((p) => p.value),
      },
    }),
  );
  await page.route(/\/api\/v1\/exchanges\/[^/]+\/body\/resp$/, (r) =>
    r.fulfill({
      status: 200,
      contentType: 'application/octet-stream',
      headers: { 'x-crosswire-length': String(Buffer.byteLength(LINES)) },
      body: Buffer.from(LINES, 'latin1'),
    }),
  );
  await page.route(/\/api\/v1\/exchanges\/[^/]+\/body\/req$/, (r) =>
    r.fulfill({ status: 404, json: { error: { code: 'not_found', message: 'no body' } } }),
  );

  await page.goto('/');
  await expect(page.getByTestId('app-shell')).toBeVisible();
  await page.getByTestId('nav-repeater').click();
  await expect(page.getByTestId('repeater-view')).toBeVisible();

  // The loaded draft's request bytes render in the editable editor.
  await expect(page.getByTestId('repeater-request').locator('.cm-content')).toContainText(XSS_CORPUS[0].value);

  await page.getByTestId('send').click();
  await expect(page.getByTestId('result-header')).toBeVisible();
  await expect(page.getByTestId('result-header')).toContainText(XSS_CORPUS[1].value);
  await expect(page.getByTestId('result-resp-body').locator('.cm-content')).toContainText(XSS_CORPUS[0].value);

  await settle(page);
  const live = await liveProbe(page);
  expect(live.pwned).toBe(0);
  expect(live.injected).toBe(0);
  expect(new URL(page.url()).origin).toBe(origin);
  expect(violations).toEqual([]);
});

test('intercept: corpus in held host/method/path and held bytes is inert', async ({ page, baseURL }) => {
  const origin = new URL(baseURL!).origin;
  const violations = watch(page, origin);
  let enabled = false;
  const held = [{
    id: HELD_ID, host: ONE_LINE, method: XSS_CORPUS[1].value, path: `/${ONE_LINE}`, size: 100, editable: true,
  }];
  const head = `POST /${ONE_LINE} HTTP/1.1\r\nHost: ${ONE_LINE}\r\nX-Evil: ${ONE_LINE}\r\n\r\n`;

  await page.route('**/api/v1/counts', (r) => r.fulfill({ status: 200, json: COUNTS }));
  await page.route('**/api/v1/scope', (r) => r.fulfill({ status: 200, json: { include: [], exclude: [] } }));
  await page.route(/\/api\/v1\/repeater\/tabs/, (r) => r.fulfill({ status: 200, json: [] }));
  await page.route(/\/api\/v1\/intercept$/, (r) =>
    r.fulfill({ status: 200, json: { enabled, rules: RULES, held: enabled ? held : [] } }),
  );
  await page.route(/\/api\/v1\/intercept\/enabled$/, (r) => {
    enabled = (r.request().postDataJSON() as { enabled: boolean }).enabled;
    r.fulfill({ status: 200, body: '' });
  });
  await page.route(/\/api\/v1\/intercept\/rules$/, (r) => r.fulfill({ status: 200, body: '' }));
  await page.route(/\/api\/v1\/intercept\/held\/[^/]+\/head$/, (r) =>
    r.fulfill({ status: 200, contentType: 'application/octet-stream', body: Buffer.from(head, 'latin1') }),
  );
  await page.route(/\/api\/v1\/intercept\/held\/[^/]+\/body$/, (r) =>
    r.fulfill({ status: 200, contentType: 'application/octet-stream', body: Buffer.from(LINES, 'latin1') }),
  );

  await page.goto('/');
  await expect(page.getByTestId('app-shell')).toBeVisible();
  await page.getByTestId('nav-intercept').click();
  await expect(page.getByTestId('intercept-view')).toBeVisible();

  await page.getByTestId('intercept-toggle').click();
  await expect.poll(() => enabled).toBe(true);
  // Production refreshes the held list from an event; re-enter the view here.
  await page.getByTestId('nav-history').click();
  await page.getByTestId('nav-intercept').click();

  const row = page.getByTestId('held-row');
  await expect(row).toHaveCount(1);
  for (const p of [XSS_CORPUS[0], XSS_CORPUS[1], XSS_CORPUS[8]]) await expect(row).toContainText(p.value);
  await row.click();

  const detail = page.getByTestId('intercept-detail');
  await expect(detail.locator('.cm-content').first()).toContainText(XSS_CORPUS[0].value);

  await settle(page);
  const live = await liveProbe(page);
  expect(live.pwned).toBe(0);
  expect(live.injected).toBe(0);
  expect(new URL(page.url()).origin).toBe(origin);
  expect(violations).toEqual([]);
});
