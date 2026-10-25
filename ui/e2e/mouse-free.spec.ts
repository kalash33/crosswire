import { test, expect, type Page, type Request } from '@playwright/test';

// §7.6 MOUSE-FREE-CORE-LOOP GATE.
//
// Drive the whole core loop from the KEYBOARD ONLY — no `click()`. The only
// non-chord keys allowed are focus moves (`locator.focus()` is programmatic
// focus, not a mouse action) and Enter/Space to activate an already-focused
// control; every state transition that Burp binds to a chord is fired through
// the chord (the Plan 5a keymap dispatch on window). Each step asserts its
// visible result.
//
// The loop: select a history exchange (focus a row + Enter), send-to-repeater
// (Ctrl+R chord), confirm the repeater tab opened, edit a byte in the editor,
// send (Ctrl+Space chord), toggle intercept (Ctrl+T chord), then forward a held
// item (Ctrl+F chord).
//
// Non-chord steps used, and why:
//  - `locator.focus()` to place keyboard focus on a row / nav button / editor
//    (the spec forbids the mouse, not programmatic focus).
//  - Enter to activate the focused primary-nav button (SPA pushState route) and
//    to open the focused history row — these are keyboard activations of a
//    focused control, not mouse clicks, and no Burp chord navigates views.
//
// No backend: `vite preview` serves the built bundle; every API call is mocked.

const COUNTS = {
  bytes_inline: 0, bytes_packs: 0, committed: 0, error: 0, exchanges: 1,
  history: 1, intruder: 0, markers: 0, repeater: 0,
};
const RULES = {
  skip_extensions: [] as string[],
  skip_out_of_scope: false,
  overflow: 'forward-newest',
  on_timeout: 'forward',
};

const EX_ID = '01HZZZZZZZZZZZZZZZZZZZZSHOP';
const HELD_ID = '01HZZZZZZZZZZZZZZZZZZZHELD';

const b64 = (s: string) => Buffer.from(s, 'latin1').toString('base64');
const REQ_HEAD = 'GET /cart HTTP/1.1\r\nHost: shop.example\r\n\r\n';
const RESP_BODY = 'HELLO-RESP-BODY';
const HELD_HEAD = 'POST /h HTTP/1.1\r\nHost: held.example\r\nContent-Length: 3\r\n\r\n';
const HELD_BODY = 'abc';

function historyRow() {
  return {
    id: EX_ID, created_at: 1_700_000_000_000, method: 'GET', host: 'shop.example',
    path: '/cart', status: 200, resp_len: RESP_BODY.length, state: 'committed', tags: ['demo'],
  };
}

function exchangeJson() {
  return {
    id: EX_ID,
    request: {
      kind: 'http1-raw', fidelity: 'raw',
      head: { type: 'http1-raw', bytes: b64(REQ_HEAD), parser_version: 1 },
      connection: { target_addr: 'shop.example', port: 443, sni: 'shop.example', tls_version: 'TLSv1.3' },
      diagnostics: [], body: { type: 'none' },
    },
    response: {
      kind: 'http1-raw', fidelity: 'raw',
      head: { type: 'http1-raw', bytes: b64('HTTP/1.1 200 OK\r\n\r\n'), parser_version: 1 },
      diagnostics: [], body: { type: 'inline', truncated_at: RESP_BODY.length },
    },
    scope_state: 'in', state: 'committed', tags: ['demo'], notes: '',
  };
}

function outcome() {
  return {
    exchange_id: EX_ID, scope_state: 'in', changes: [], skipped: [],
    cert_override: false, ttfb_ms: 7, total_ms: 21, redirects: [],
  };
}

const HELD = {
  id: HELD_ID, host: 'held.example', method: 'POST', path: '/h', size: 60, editable: true,
};

interface Harness {
  sends: Request[];
  forwards: { id: string; body: string | null }[];
  drops: string[];
  enabledPuts: { enabled: boolean }[];
}

async function mockApi(page: Page): Promise<Harness> {
  const h: Harness = { sends: [], forwards: [], drops: [], enabledPuts: [] };
  let intercept = false;

  await page.route('**/api/v1/counts', (r) => r.fulfill({ status: 200, json: COUNTS }));
  await page.route('**/api/v1/scope', (r) =>
    r.fulfill({ status: 200, json: { include: [], exclude: [] } }),
  );

  // Repeater tab drafts: start empty; autosave PUT / DELETE accepted.
  await page.route(/\/api\/v1\/repeater\/tabs$/, (r) => r.fulfill({ status: 200, json: [] }));
  await page.route(/\/api\/v1\/repeater\/tabs\/[^/]+$/, (r) => r.fulfill({ status: 200, body: '' }));
  await page.route(/\/api\/v1\/repeater\/send$/, (r) => {
    h.sends.push(r.request());
    r.fulfill({ status: 200, json: outcome() });
  });

  // Intercept status / toggle / actions.
  await page.route(/\/api\/v1\/intercept$/, (r) =>
    r.fulfill({ status: 200, json: { enabled: intercept, rules: RULES, held: intercept ? [HELD] : [] } }),
  );
  await page.route(/\/api\/v1\/intercept\/enabled$/, (r) => {
    const b = r.request().postDataJSON() as { enabled: boolean };
    h.enabledPuts.push(b);
    intercept = b.enabled;
    r.fulfill({ status: 200, body: '' });
  });
  await page.route(/\/api\/v1\/intercept\/held\/[^/]+\/forward$/, (r) => {
    const id = r.request().url().split('/').slice(-2)[0];
    h.forwards.push({ id, body: r.request().postData() });
    intercept = intercept; // held clears on next refresh
    r.fulfill({ status: 200, body: '' });
  });
  await page.route(/\/api\/v1\/intercept\/held\/[^/]+\/drop$/, (r) => {
    h.drops.push(r.request().url().split('/').slice(-2)[0]);
    r.fulfill({ status: 200, body: '' });
  });
  await page.route(/\/api\/v1\/intercept\/held\/[^/]+\/head$/, (r) =>
    r.fulfill({ status: 200, contentType: 'application/octet-stream', body: Buffer.from(HELD_HEAD, 'latin1') }),
  );
  await page.route(/\/api\/v1\/intercept\/held\/[^/]+\/body$/, (r) =>
    r.fulfill({ status: 200, contentType: 'application/octet-stream', body: Buffer.from(HELD_BODY, 'latin1') }),
  );

  // Exchange detail + bodies (register before the list route).
  await page.route(/\/api\/v1\/exchanges\/[^/]+\/body\/req$/, (r) =>
    r.fulfill({ status: 404, json: { error: { code: 'not_found', message: 'no body' } } }),
  );
  await page.route(/\/api\/v1\/exchanges\/[^/]+\/body\/resp$/, (r) =>
    r.fulfill({
      status: 200,
      contentType: 'application/octet-stream',
      headers: { 'x-crosswire-length': String(RESP_BODY.length) },
      body: Buffer.from(RESP_BODY, 'latin1'),
    }),
  );
  await page.route(/\/api\/v1\/exchanges\/[^/?]+$/, (r) =>
    r.fulfill({ status: 200, json: exchangeJson() }),
  );
  await page.route(/\/api\/v1\/exchanges(\?.*)?$/, (r) =>
    r.fulfill({ status: 200, json: { items: [historyRow()], next_cursor: null } }),
  );

  return h;
}

/** Keyboard-activate a focused primary-nav button (SPA pushState, no mouse). */
async function navByKeyboard(page: Page, testid: string): Promise<void> {
  await page.getByTestId(testid).focus();
  await page.keyboard.press('Enter');
}

test('mouse-free core loop: history -> repeater -> send -> intercept via Burp chords', async ({
  page,
}) => {
  const h = await mockApi(page);

  await page.goto('/');
  await expect(page.getByTestId('app-shell')).toBeVisible();
  await expect(page.getByTestId('history-view')).toBeVisible();
  await expect(page.getByTestId('history-row')).toHaveCount(1);

  // ---- 1. Select a history exchange: focus the row, open it with Enter. ------
  const row = page.getByTestId('history-row').first();
  await row.focus();
  await page.keyboard.press('Enter');
  await expect(page.getByTestId('exchange-detail')).toBeVisible();

  // ---- 2. send-to-repeater (Ctrl+R chord) -> seeds a tab + navigates. --------
  await page.keyboard.press('Control+r');
  await expect(page.getByTestId('repeater-view')).toBeVisible();
  await expect(page.getByTestId('repeater-tab')).toHaveCount(1);
  // The seeded tab carries the exchange's raw request bytes.
  const editor = page.locator('.cm-content').first();
  await expect(editor).toContainText('GET /cart HTTP/1.1');

  // ---- 3. Edit a byte in the repeater editor (keyboard). --------------------
  await editor.focus();
  await page.keyboard.press('Home');
  await page.keyboard.type('Z');
  await expect(editor).toContainText('ZGET /cart');

  // ---- 4. send (Ctrl+Space chord) from inside the editor (Burp behavior). ----
  const sendPosted = page.waitForRequest(/\/api\/v1\/repeater\/send$/);
  await page.keyboard.press('Control+Space');
  await sendPosted;
  await expect(page.getByTestId('result-header')).toBeVisible();
  // The sent bytes carry the keyboard edit.
  const sent = h.sends.at(-1)!;
  const body = JSON.parse(sent.postData() ?? '{}') as { raw: string; host: string };
  const decoded = Buffer.from(body.raw, 'base64').toString('latin1');
  expect(body.host).toBe('shop.example');
  expect(decoded.startsWith('ZGET /cart')).toBe(true);

  // ---- 5. toggle-intercept (Ctrl+T chord) from anywhere. --------------------
  await expect(page.getByTestId('status-intercept')).toContainText('OFF');
  await page.keyboard.press('Control+t');
  await expect.poll(() => h.enabledPuts).toEqual([{ enabled: true }]);
  await expect(page.getByTestId('status-intercept')).toContainText('ON');

  // ---- 6. Go to intercept, focus the held item, forward it (Ctrl+F chord). ---
  await navByKeyboard(page, 'nav-intercept');
  await expect(page.getByTestId('intercept-view')).toBeVisible();
  await expect(page.getByTestId('held-row')).toHaveCount(1);
  await page.getByTestId('held-row').first().focus();
  // Wait for the held bytes to load so forward sends the real request.
  await expect(page.locator('.cm-content').first()).toContainText('POST /h');
  await page.keyboard.press('Control+f');
  await expect.poll(() => h.forwards.length).toBe(1);
  expect(h.forwards[0].id).toBe(HELD_ID);
});

test('mouse-free: drop a held item via the Ctrl+D chord', async ({ page }) => {
  const h = await mockApi(page);

  await page.goto('/');
  await expect(page.getByTestId('app-shell')).toBeVisible();

  // READINESS: the global toggle-intercept handler no-ops until the intercept
  // status has loaded (the status bar shows "…" until then, "OFF"/"ON" after).
  // Waiting for "OFF" guarantees `intercept.loaded` is true, so the Ctrl+T chord
  // cannot land before the handler can act — the fix for the earlier race.
  await expect(page.getByTestId('status-intercept')).toContainText('OFF');

  // Turn intercept on with the chord, then open the intercept view.
  await page.keyboard.press('Control+t');
  await expect.poll(() => h.enabledPuts).toEqual([{ enabled: true }]);
  await page.getByTestId('nav-intercept').focus();
  await page.keyboard.press('Enter');
  await expect(page.getByTestId('held-row')).toHaveCount(1);

  await page.getByTestId('held-row').first().focus();
  await page.keyboard.press('Control+d');
  await expect.poll(() => h.drops.length).toBe(1);
  expect(h.drops[0]).toBe(HELD_ID);
});
