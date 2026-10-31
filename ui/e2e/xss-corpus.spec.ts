import { test, expect, type Page } from '@playwright/test';
import { XSS_CORPUS } from './fixtures/xss-corpus';
import { watch, liveProbe } from './fixtures/xss-detect';
import { apply, CODECS, type Direction } from '../src/lib/decode/codecs';
import { fromTextareaText } from '../src/lib/decode/textarea';
import { bytesToLatin1 } from '../src/lib/editor/encoding';

// Text-rendering half of the XSS gate: captured bytes are attacker-controlled,
// so seed the corpus into every rendered field and prove nothing executes.

const COUNTS = {
  bytes_inline: 0, bytes_packs: 0, committed: 0, error: 0, exchanges: 0,
  history: 0, intruder: 0, markers: 0, repeater: 0,
};
const ID = '01HZZZZZZZZZZZZZZZZZZZZXSS';
const b64 = (s: string) => Buffer.from(s, 'utf8').toString('base64');

const ALL = XSS_CORPUS.map((p) => p.value).join(' ');
const ONE_LINE_ALL = ALL.replace(/[\r\n]/g, '');
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

// U1: below ~760px of detail-pane width the detail shows one message at a
// time behind a Request/Response switch (default Response); side by side,
// both are always rendered and these are no-ops.
async function showSide(page: Page, side: 'req' | 'resp') {
  const sw = page.getByTestId(side === 'req' ? 'detail-side-req' : 'detail-side-resp');
  if ((await sw.count()) > 0) await sw.click();
}
const showRequestSide = (page: Page) => showSide(page, 'req');
const showResponseSide = (page: Page) => showSide(page, 'resp');

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
    // Navigation target is FULFILLED (not aborted) with a real document so the
    // main frame actually commits cross-origin and `framenavigated` fires every
    // run. Aborting the navigation left the commit racy — the frame committed
    // (and fired framenavigated) only sometimes, which made this gate flaky.
    await page.route('http://nav.invalid/**', (r) =>
      r.fulfill({ status: 200, contentType: 'text/html', body: '<!doctype html><title>x</title>' }),
    );
    await page.goto('/e2e/fixture/index.html');
    const violations = watch(page, origin);
    await page.evaluate(() => {
      const img = document.createElement('img');
      img.setAttribute('onerror', 'alert(1)');
      img.src = 'http://evil.invalid/x.png';
      document.body.appendChild(img);
    });
    await expect.poll(() => violations.some((v) => v.startsWith('dialog'))).toBe(true);
    await expect.poll(() => violations.some((v) => v.startsWith('off-origin request:'))).toBe(true);
    await page.evaluate(() => { location.href = 'http://nav.invalid/nav'; });
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
  await showRequestSide(page);
  await expect(page.getByTestId('detail-req-body').locator('.cm-content')).toContainText(
    XSS_CORPUS[1].value,
  );
  await showResponseSide(page);
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

test('typed_tag_and_notes_payloads_render_as_text', async ({ page, baseURL }) => {
  const origin = new URL(baseURL!).origin;
  // Stateful mock: PATCH records the body and answers 200 with an EMPTY body (like the real route).
  let tags: string[] = [];
  let notes = '';
  const patches: { tags?: string[]; notes?: string }[] = [];
  await page.route('**/api/v1/counts', (r) => r.fulfill({ status: 200, json: COUNTS }));
  await page.route('**/api/v1/scope', (r) => r.fulfill({ status: 200, json: { include: [], exclude: [] } }));
  await page.route(/\/api\/v1\/exchanges\/[^/]+\/body\/(req|resp)$/, (r) =>
    r.fulfill({ status: 404, json: { error: { code: 'not_found', message: 'x' } } }),
  );
  await page.route(/\/api\/v1\/exchanges\/[^/?]+$/, (r) => {
    if (r.request().method() === 'PATCH') {
      const body = r.request().postDataJSON() as { tags?: string[]; notes?: string };
      patches.push(body);
      if (body.tags) tags = body.tags;
      if (body.notes !== undefined) notes = body.notes;
      return r.fulfill({ status: 200, body: '' });
    }
    return r.fulfill({ status: 200, json: { ...DETAIL, request: { ...DETAIL.request, body: { type: 'none' } }, response: null, tags, notes } });
  });
  await page.route(/\/api\/v1\/exchanges(\?.*)?$/, (r) =>
    r.fulfill({ status: 200, json: { items: [{ ...ROW, host: 'h.example', path: '/', tags }], next_cursor: null } }),
  );
  const violations = watch(page, origin);
  await page.goto('/');
  const startUrl = page.url();
  await page.getByTestId('history-row').locator('td.open').first().click();
  await expect(page.getByTestId('exchange-detail')).toBeVisible();

  // Tags reject over-64-char values and control characters (client guard). Payloads over the
  // limit are typed as their first 64 chars (trimmed) so every payload still drives the chip path.
  // The corpus has no CR/LF, so no control-character handling is needed on the tag path.
  const typedTags: string[] = [];
  for (const p of XSS_CORPUS) {
    const tag = p.value.slice(0, 64).trim();
    typedTags.push(tag);
    const before = patches.length;
    await page.getByTestId('tag-input').fill(tag);
    await page.getByTestId('tag-input').press('Enter');
    await expect.poll(() => patches.length, { message: p.name }).toBe(before + 1);
    expect(patches[before].tags).toEqual(typedTags); // verbatim, full list
    await expect(page.getByTestId('tag-error')).toHaveCount(0);
    // Let the save settle (row patch / detail refresh) before typing the next tag.
    await expect(page.getByTestId('tag-chip')).toHaveCount(typedTags.length);
    await expect(page.getByTestId('tag-input')).toHaveValue('');
  }
  const chips = page.getByTestId('tag-chip');
  await expect(chips).toHaveCount(XSS_CORPUS.length);
  for (let i = 0; i < typedTags.length; i++) {
    // chip text is exactly the tag plus the remove button's glyph
    const text = (await chips.nth(i).textContent()) ?? '';
    expect(text, `chip ${i}`).toBe(typedTags[i] + '×');
    expect(await chips.nth(i).locator('[data-testid="tag-remove"]').count()).toBe(1);
  }
  const rowTags = page.getByTestId('history-row').locator('td.tags');
  for (const t of typedTags) await expect(rowTags).toContainText(t);

  // Notes: a textarea normalizes CR to LF, so the oracle is the value the textarea holds.
  const notesEl = page.getByTestId('notes-input');
  for (const p of XSS_CORPUS) {
    const before = patches.length;
    await notesEl.fill(p.value);
    const held = await notesEl.inputValue();
    expect(held).toBe(p.value.replace(/\r\n?/g, '\n'));
    await notesEl.blur();
    await expect.poll(() => patches.length, { message: p.name }).toBeGreaterThan(before);
    expect(patches[patches.length - 1]).toEqual({ notes: held });
  }

  await page.mouse.move(5, 5);
  await page.mouse.move(300, 300);
  await page.waitForTimeout(300);
  const live = await liveProbe(page);
  expect(live.pwned).toBe(0);
  expect(live.injected).toBe(0);
  expect(page.url()).toBe(startUrl);
  expect(violations).toEqual([]);
});

test('decoder_outputs_are_text_for_every_payload', async ({ page, baseURL }) => {
  const origin = new URL(baseURL!).origin;
  await page.route('**/api/v1/counts', (r) => r.fulfill({ status: 200, json: COUNTS }));
  await page.route(/\/api\/v1\/exchanges(\?.*)?$/, (r) =>
    r.fulfill({ status: 200, json: { items: [], next_cursor: null } }),
  );
  const violations = watch(page, origin);
  await page.goto('/');
  // The chord only works once the app has hydrated; retry the press until the pane appears.
  await expect(page.getByTestId('app-root')).toBeVisible();
  await expect(async () => {
    await page.keyboard.press('Control+Shift+D');
    await expect(page.getByTestId('decoder-pane')).toBeVisible({ timeout: 1000 });
  }).toPass();
  // U1 D6: the decoder is the /decoder route, so the chord itself navigates
  // there; payloads must not navigate anywhere after that.
  const startUrl = page.url();
  expect(new URL(startUrl).pathname).toBe('/decoder');
  const input = page.getByTestId('decoder-input');
  await page.getByTestId('decoder-step-add').click();
  const codecSel = page.getByTestId('decoder-step-0-codec');
  const dirSel = page.getByTestId('decoder-step-0-dir');
  const out = page.getByTestId('decoder-output-0');
  const dirs: Direction[] = ['encode', 'decode'];

  for (const p of XSS_CORPUS) {
    await input.fill(p.value);
    // Oracle is computed from what the textarea ACTUALLY holds (CR normalization cannot desync it).
    const bytes = fromTextareaText(await input.inputValue()).bytes;
    for (const codec of CODECS) {
      for (const dir of dirs) {
        await codecSel.selectOption(codec);
        await dirSel.selectOption(dir);
        const expected = bytesToLatin1(apply(codec, dir, bytes).bytes);
        await expect(out, `${p.name} ${codec} ${dir}`).toHaveText(expected, { useInnerText: false });
        expect(await out.textContent()).toBe(expected);
      }
    }
    const live = await liveProbe(page);
    expect(live.pwned, p.name).toBe(0);
    expect(live.injected, p.name).toBe(0);
  }

  // A payload that is also a decodable JWT-shaped string renders in the JWT view as text.
  const u = (s: string) => Buffer.from(s).toString('base64url');
  const jwtish = `${u(XSS_CORPUS[0].value)}.${u(XSS_CORPUS[1].value)}.${u(XSS_CORPUS[2].value)}`;
  await input.fill(jwtish);
  await expect(page.getByTestId('decoder-jwt')).toBeVisible();
  await expect(page.getByTestId('decoder-jwt-banner')).toHaveText('signature not verified');
  const jwtText = (await page.getByTestId('decoder-jwt').textContent()) ?? '';
  expect(jwtText).toContain(XSS_CORPUS[0].value);
  expect(jwtText).toContain(XSS_CORPUS[1].value);

  await page.mouse.move(5, 5);
  await page.mouse.move(300, 300);
  await page.waitForTimeout(300);
  const live = await liveProbe(page);
  expect(live.pwned).toBe(0);
  expect(live.injected).toBe(0);
  expect(page.url()).toBe(startUrl);
  expect(violations).toEqual([]);
});

// ---- U1 surfaces ---------------------------------------------------------
// The redesign added new places where captured or server-supplied values are
// shown: the Headers tab, the detail title line, context menus, toasts,
// repeater tab titles, the Start panel, Settings and the seeded Decoder route.

async function mockU1(page: Page, scopePut: () => { status: number; json?: unknown }) {
  await page.addInitScript(() => {
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: { writeText: async () => undefined },
    });
  });
  await mockApi(page);
  await page.route('**/api/v1/scope', (r) => {
    if (r.request().method() === 'PUT') {
      const res = scopePut();
      return r.fulfill({ status: res.status, json: res.json ?? {} });
    }
    return r.fulfill({ status: 200, json: { include: [], exclude: [] } });
  });
  await page.route(/\/api\/v1\/repeater\/tabs$/, (r) => r.fulfill({ status: 200, json: [] }));
  await page.route(/\/api\/v1\/repeater\/tabs\/[^/]+$/, (r) => r.fulfill({ status: 200, body: '' }));
}

async function settle(page: Page) {
  await page.mouse.move(5, 5);
  await page.mouse.move(300, 300);
  await page.keyboard.press('Tab');
  await page.waitForTimeout(300);
}

async function assertInert(page: Page, violations: string[], startUrl: string) {
  await settle(page);
  const live = await liveProbe(page);
  expect(live.pwned).toBe(0);
  expect(live.injected).toBe(0);
  expect(page.url()).toBe(startUrl);
  expect(violations).toEqual([]);
}

test('U1: headers tab, detail title, context menu and toasts render the corpus as text', async ({ page, baseURL }) => {
  test.setTimeout(120_000); // one pass per payload over several surfaces
  const origin = new URL(baseURL!).origin;
  let puts = 0;
  await mockU1(page, () =>
    ++puts === 1
      ? { status: 400, json: { error: { code: 'invalid', message: ONE_LINE_ALL } } }
      : { status: 200 },
  );
  const violations = watch(page, origin);
  await page.goto('/');
  const startUrl = page.url();

  const row = page.getByTestId('history-row');
  await expect(row).toHaveCount(1);
  await row.locator('td.open').first().click();
  const detail = page.getByTestId('exchange-detail');
  await expect(detail).toBeVisible();

  // Detail title line: METHOD /path and the host, verbatim.
  const title = detail.locator('h2.title');
  const host = detail.locator('.meta .host');
  for (const p of XSS_CORPUS) {
    await expect(title, p.name).toContainText(p.value);
    await expect(host, p.name).toContainText(p.value);
  }
  expect(await title.getAttribute('title')).toContain(XSS_CORPUS[1].value);

  // Headers tab, both sides: every header name and value cell is text.
  for (const side of ['req', 'resp'] as const) {
    await showSide(page, side);
    const pane = page.getByTestId(side === 'req' ? 'detail-request' : 'detail-response');
    await pane.getByTestId('detail-tab-headers').click();
    const table = page.getByTestId(`detail-${side}-headers`);
    await expect(table).toBeVisible();
    for (const [i, p] of XSS_CORPUS.entries()) {
      const name = `X-${i}${p.value.replace(/[\s:\r\n]/g, '')}`;
      await expect(table.locator('th', { hasText: name }).first(), `${side} name ${p.name}`).toHaveText(name);
      await expect(table, `${side} value ${p.name}`).toContainText(p.value);
    }
    if (side === 'req') await expect(page.getByTestId('headers-start')).toContainText(XSS_CORPUS[0].value);
    await pane.getByTestId('detail-tab-raw').click();
  }

  // Row context menu over a hostile row: labels are fixed text.
  await row.first().click({ button: 'right' });
  const menu = page.getByTestId('row-menu');
  await expect(menu).toBeVisible();
  const labels = (await menu.innerText()).split('\n').map((s) => s.trim()).filter(Boolean);
  for (const l of labels) expect(l, 'menu label').not.toMatch(/[<>"]/);
  // Add host to scope: the first PUT fails with a hostile server message
  // (error toast + inline error), the second succeeds (success toast).
  await menu.getByTestId('row-menu-add-host').click();
  const errToast = page.locator('[data-testid="toast"][data-kind="error"]');
  await expect(errToast.first()).toContainText(XSS_CORPUS[1].value);
  await expect(page.getByTestId('scope-action-error')).toContainText(XSS_CORPUS[1].value);
  await page.getByTestId('selection-toolbar').getByTestId('add-host-to-scope').click();
  const okToast = page.locator('[data-testid="toast"][data-kind="success"]');
  await expect(okToast.filter({ hasText: 'to scope.' })).toContainText(XSS_CORPUS[0].value);

  // A copy toast for a hostile URL (the toast wording is fixed).
  await page.getByTestId('copy-menu').click();
  await page.getByTestId('copy-url').click();
  await expect(okToast.filter({ hasText: 'Copied URL.' })).toHaveCount(1);

  // Bulk tag with a hostile tag: the toast quotes it as text.
  const tag = XSS_CORPUS[4].value; // svg-onload, under the 64-char tag limit
  await row.first().locator('input[type="checkbox"]').check();
  await page.getByTestId('bulk-tag-input').fill(tag);
  await page.getByTestId('bulk-tag-apply').click();
  await expect(okToast.filter({ hasText: 'Tagged 1 exchange' })).toContainText(tag);

  await assertInert(page, violations, startUrl);
});

test('U1: repeater tab title and the seeded Decoder route render the corpus as text', async ({ page, baseURL }) => {
  const origin = new URL(baseURL!).origin;
  await mockU1(page, () => ({ status: 200 }));
  const violations = watch(page, origin);
  await page.goto('/');

  // Decode selection over the hostile response body seeds /decoder.
  await page.getByTestId('history-row').locator('td.open').first().click();
  const body = page.getByTestId('detail-resp-body');
  await expect(body).toContainText(XSS_CORPUS[0].value);
  await body.locator('.cm-content').focus();
  await body.evaluate((el) => {
    const range = document.createRange();
    range.selectNodeContents(el.querySelector('.cm-content') ?? el);
    const sel = window.getSelection()!;
    sel.removeAllRanges();
    sel.addRange(range);
  });
  await page.getByTestId('decode-selection').click();
  await expect(page).toHaveURL(/\/decoder$/);
  await expect(page.getByTestId('decoder-input')).toHaveValue(RESP_BODY);
  await page.getByTestId('decoder-step-add').click();
  await page.getByTestId('decoder-step-0-codec').selectOption('url');
  await page.getByTestId('decoder-step-0-dir').selectOption('decode');
  await expect(page.getByTestId('decoder-output-0')).toContainText(XSS_CORPUS[0].value);
  await assertInert(page, violations, page.url());

  // Send to Repeater: the tab title is derived from the hostile Host header.
  await page.getByTestId('nav-history').click();
  await page.getByTestId('history-row').locator('td.open').first().click();
  await page.getByTestId('selection-toolbar').getByTestId('send-to-repeater').click();
  await expect(page).toHaveURL(/\/repeater$/);
  const tab = page.getByTestId('repeater-tab');
  await expect(tab).toHaveCount(1);
  await expect(tab.locator('.text')).toContainText(`POST ${XSS_CORPUS[0].value}`);
  await expect(page.getByTestId('repeater-request').locator('.cm-content')).toContainText(XSS_CORPUS[1].value);
  await assertInert(page, violations, page.url());
});

// Hostile health values: proxy_addr must be rejected by parseProxyAddr (the
// default shows; nothing navigates), version/schema/format render as text.
const HOSTILE_ADDRS = [
  ...XSS_CORPUS.map((p) => p.value),
  'evil.invalid:80/x',
  '127.0.0.1:8080 --proxy-server=evil.invalid',
  'javascript:alert(1)//:80',
  'evil.invalid:80\r\nX: y',
  'http://evil.invalid:80',
];

test('U1: Start panel, status bar and Settings reject or quote hostile health values', async ({ page, baseURL }) => {
  test.setTimeout(120_000); // one pass per payload over several surfaces
  const origin = new URL(baseURL!).origin;
  const violations = watch(page, origin);
  await page.addInitScript(() => {
    (window as unknown as { __opened: unknown[] }).__opened = [];
    window.open = ((...args: unknown[]) => {
      (window as unknown as { __opened: unknown[] }).__opened.push(args);
      return null;
    }) as typeof window.open;
  });
  let health: Record<string, unknown> = {};
  await page.route('**/api/v1/health', (r) => r.fulfill({ status: 200, json: health }));
  await page.route('**/api/v1/counts', (r) => r.fulfill({ status: 200, json: COUNTS }));
  await page.route('**/api/v1/scope', (r) => r.fulfill({ status: 200, json: { include: [], exclude: [] } }));
  await page.route(/\/api\/v1\/exchanges(\?.*)?$/, (r) =>
    r.fulfill({ status: 200, json: { items: [], next_cursor: null } }),
  );

  for (const [i, addr] of HOSTILE_ADDRS.entries()) {
    const p = XSS_CORPUS[i % XSS_CORPUS.length];
    health = { status: 'ok', schema_version: p.value, format_version: addr, version: p.value, proxy_addr: addr };
    await page.goto('/');
    const startUrl = page.url();
    const panel = page.getByTestId('start-panel');
    await expect(panel, addr).toBeVisible();
    await expect(page.getByTestId('start-browser-command')).toHaveText(
      'crosswire browser --project PROJECT_DIR --proxy 127.0.0.1:8080',
    );
    // A rejected address is not reported at all in the chrome (no guess).
    await expect(page.getByTestId('status-proxy')).toHaveCount(0);
    await expect(page.getByTestId('topbar-proxy')).toHaveCount(0);
    await page.getByTestId('start-open-ca').click();
    const opened = await page.evaluate(() => (window as unknown as { __opened: unknown[][] }).__opened);
    expect(opened, addr).toEqual([['http://127.0.0.1:8080/', '_blank', 'noopener']]);

    await page.getByTestId('nav-settings').click();
    await expect(page.getByTestId('settings-about')).toBeVisible();
    await expect(page.getByTestId('about-schema-version')).toHaveText(p.value);
    await expect(page.getByTestId('about-format-version')).toHaveText(addr.replace(/\r\n/g, ' '));
    if (p.value.length <= 64) await expect(page.getByTestId('about-version')).toHaveText(p.value);
    else await expect(page.getByTestId('about-version')).toHaveCount(0);
    await expect(page.getByTestId('settings-proxy')).toContainText('127.0.0.1:8080');
    await expect(page.getByTestId('settings-ca')).toContainText('http://127.0.0.1:8080/');
    await expect(page.getByTestId('settings-connection')).not.toContainText('evil');

    const live = await liveProbe(page);
    expect(live.pwned, addr).toBe(0);
    expect(live.injected, addr).toBe(0);
    expect(new URL(page.url()).origin).toBe(new URL(startUrl).origin);
  }
  expect(violations).toEqual([]);
});
