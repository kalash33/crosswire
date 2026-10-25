import { test, expect, type Page } from '@playwright/test';
import { XSS_CORPUS } from './fixtures/xss-corpus';
import { watch, liveProbe } from './fixtures/xss-detect';

// Text-rendering half of the XSS gate: captured bytes are attacker-controlled,
// so seed the corpus into every rendered field and prove nothing executes.

const COUNTS = {
  bytes_inline: 0, bytes_packs: 0, committed: 0, error: 0, exchanges: 0,
  history: 0, intruder: 0, markers: 0, repeater: 0,
};
const ID = '01HZZZZZZZZZZZZZZZZZZZZXSS';
const b64 = (s: string) => Buffer.from(s, 'utf8').toString('base64');

const ALL = XSS_CORPUS.map((p) => p.value).join(' ');
// Header names and values both carry payloads (spaces/colons are structural, so
// use a payload with neither as the name).
const HEADERS = XSS_CORPUS.map((p, i) => `X-${i}${p.value.replace(/[\s:\r\n]/g, '')}: ${p.value.replace(/[\r\n]/g, '')}`).join('\r\n');
const REQ_HEAD = `POST /${ALL.replace(/[\r\n]/g, '')} HTTP/1.1\r\nHost: ${ALL.replace(/[\r\n]/g, '')}\r\n${HEADERS}\r\n\r\n`;
const RESP_HEAD = `HTTP/1.1 200 OK\r\n${HEADERS}\r\nContent-Length: ${ALL.length}\r\n\r\n`;
const REQ_BODY = XSS_CORPUS.map((p) => p.value).join('\n');
const RESP_BODY = ALL;

const ROW = {
  id: ID,
  created_at: 1_700_000_000_000,
  method: 'POST',
  host: ALL,
  path: `/${ALL}`,
  status: 200,
  resp_len: RESP_BODY.length,
  state: 'committed',
  tags: XSS_CORPUS.map((p) => p.value),
};

const DETAIL = {
  id: ID,
  request: {
    kind: 'http1-raw',
    fidelity: 'raw',
    head: { type: 'http1-raw', bytes: b64(REQ_HEAD), parser_version: 1 },
    diagnostics: [],
    body: { type: 'inline', truncated_at: null },
  },
  response: {
    kind: 'http1-raw',
    fidelity: 'raw',
    head: { type: 'http1-raw', bytes: b64(RESP_HEAD), parser_version: 1 },
    diagnostics: [],
    body: { type: 'inline', truncated_at: null },
  },
  scope_state: 'in',
  state: 'committed',
  tags: XSS_CORPUS.map((p) => p.value),
  notes: ALL,
};

async function mockApi(page: Page) {
  await page.route('**/api/v1/counts', (r) => r.fulfill({ status: 200, json: COUNTS }));
  await page.route(/\/api\/v1\/exchanges\/[^/?]+$/, (r) => r.fulfill({ status: 200, json: DETAIL }));
  const body = (text: string) => (r: import('@playwright/test').Route) =>
    r.fulfill({
      status: 200,
      contentType: 'application/octet-stream',
      headers: { 'x-crosswire-length': String(Buffer.byteLength(text)) },
      body: Buffer.from(text, 'utf8'),
    });
  await page.route(/\/api\/v1\/exchanges\/[^/]+\/body\/req$/, body(REQ_BODY));
  await page.route(/\/api\/v1\/exchanges\/[^/]+\/body\/resp$/, body(RESP_BODY));
  await page.route(/\/api\/v1\/exchanges(\?.*)?$/, (r) =>
    r.fulfill({ status: 200, json: { items: [ROW], next_cursor: null } }),
  );
}
test.describe('detector self-test (the harness can fail)', () => {
  // Proves the listeners (fixtures/xss-detect.ts) really catch execution, using a throwaway page
  // rather than any production render path (no {@html} anywhere in ui/src).
  test('catches a dialog, an off-origin request and a navigation', async ({ page, baseURL }) => {
    const origin = new URL(baseURL!).origin;
    await page.route('http://evil.invalid/**', (r) => r.abort());
    await page.goto('/e2e/fixture/index.html');
    const violations = watch(page, origin);
    await page.evaluate(() => {
      const img = document.createElement('img');
      img.setAttribute('onerror', 'alert(1)');
      img.src = 'http://evil.invalid/x.png';
      document.body.appendChild(img);
    });
    await expect.poll(() => violations.some((v) => v.startsWith('dialog'))).toBe(true);
    expect(violations.some((v) => v.startsWith('off-origin request:'))).toBe(true);
    await page.evaluate(() => { location.href = 'http://evil.invalid/nav'; });
    await expect.poll(() => violations.some((v) => v.startsWith('navigated away'))).toBe(true);
  });
});

test('corpus in list and detail renders as inert text', async ({ page, baseURL }) => {
  const origin = new URL(baseURL!).origin;
  await mockApi(page);
  const violations = watch(page, origin);
  await page.goto('/');
  const startUrl = page.url();

  await expect(page.getByTestId('history-row')).toHaveCount(1);
  const row = page.getByTestId('history-row');
  // Payloads appear literally in the list (host, path, tags), not as elements.
  for (const p of [XSS_CORPUS[0], XSS_CORPUS[1], XSS_CORPUS[8]]) {
    await expect(row).toContainText(p.value);
  }

  await row.first().hover();
  await row.first().click();
  await expect(page.getByTestId('exchange-detail')).toBeVisible();
  await expect(page.getByTestId('detail-resp-body').locator('.cm-content')).toContainText(
    XSS_CORPUS[0].value,
  );
  await expect(page.getByTestId('detail-req-body').locator('.cm-content')).toContainText(
    XSS_CORPUS[1].value,
  );
  await expect(page.getByTestId('detail-resp-head')).toBeVisible();

  // Hover/focus-driven handlers would fire on interaction; poke the page too.
  await page.mouse.move(5, 5);
  await page.mouse.move(300, 300);
  await page.keyboard.press('Tab');
  await page.waitForTimeout(500);

  // No payload became live DOM anywhere in the app.
  const live = await liveProbe(page);
  expect(live.pwned).toBe(0);
  expect(live.injected).toBe(0);
  expect(page.url()).toBe(startUrl);
  expect(violations).toEqual([]);
});
