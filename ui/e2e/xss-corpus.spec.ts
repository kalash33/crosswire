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
  const startUrl = page.url();
  // The chord only works once the app has hydrated; retry the press until the pane appears.
  await expect(page.getByTestId('app-root')).toBeVisible();
  await expect(async () => {
    await page.keyboard.press('Control+Shift+D');
    await expect(page.getByTestId('decoder-pane')).toBeVisible({ timeout: 1000 });
  }).toPass();
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
