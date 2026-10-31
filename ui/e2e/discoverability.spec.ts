import { test, expect, type Page } from '@playwright/test';
import { mockApp, mockEvents } from './fixtures/app-mock';

// U1 D2/D4 discoverability: every History action has a visible home (rail,
// selection toolbar, row context menu, detail title line), and the redesigned
// detail behaves as reviewed (side switch, lone-CR header, action target,
// per-side tab ids). Backend-free: the API is mocked per test.

const b64 = (s: string) => Buffer.from(s, 'latin1').toString('base64');
const BODY = 'SGVsbG8gd29ybGQ=';
const ids = ['01HDISCOVERDISCOVERDISCOV1', '01HDISCOVERDISCOVERDISCOV2', '01HDISCOVERDISCOVERDISCOV3'];

const msg = (head: string, body: unknown) => ({
  kind: 'http1-raw', fidelity: 'raw', diagnostics: [],
  head: { type: 'http1-raw', bytes: b64(head), parser_version: 1 }, body,
});

interface Opts {
  /** Request head for every exchange (default a plain GET). */
  reqHead?: string;
}

async function mock(page: Page, opts: Opts = {}) {
  await mockApp(page, {
    rows: () =>
      ids.map((id, i) => ({
        // Newest first, so table order is ids order.
        id, created_at: 1_700_000_000_000 - i, method: 'GET', host: 'shop.example',
        path: `/p/${i + 1}?q=1`, status: 200, resp_len: BODY.length, state: 'committed', tags: [],
      })),
  });
  await mockEvents(page);
  const reqHead = opts.reqHead ?? 'GET /p/1?q=1 HTTP/1.1\r\nHost: shop.example\r\nAccept: */*\r\n\r\n';
  await page.route(/\/api\/v1\/exchanges\/[^/?]+$/, (r) => {
    const id = decodeURIComponent(new URL(r.request().url()).pathname.split('/').pop()!);
    return r.fulfill({
      status: 200,
      json: {
        id,
        request: {
          ...msg(reqHead, { type: 'none' }),
          connection: { target_addr: 'shop.example', port: 443, tls_version: 'TLSv1.3', sni: 'shop.example' },
        },
        response: msg('HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\n', { type: 'inline', truncated_at: null }),
        scope_state: 'in', state: 'committed', tags: [], notes: '',
      },
    });
  });
  await page.route(/\/api\/v1\/exchanges\/[^/]+\/body\/req$/, (r) =>
    r.fulfill({ status: 404, json: { error: { code: 'not_found', message: 'none' } } }),
  );
  await page.route(/\/api\/v1\/exchanges\/[^/]+\/body\/resp$/, (r) =>
    r.fulfill({
      status: 200, contentType: 'application/octet-stream',
      headers: { 'x-crosswire-length': String(BODY.length) }, body: Buffer.from(BODY, 'latin1'),
    }),
  );
  await page.route(/\/api\/v1\/repeater\/tabs\/[^/]+$/, (r) => r.fulfill({ status: 200, body: '' }));
}

async function clipboardStub(page: Page) {
  await page.addInitScript(() => {
    (window as unknown as { __clip: string[] }).__clip = [];
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: { writeText: async (t: string) => void (window as unknown as { __clip: string[] }).__clip.push(t) },
    });
  });
}
const clip = (page: Page) => page.evaluate(() => (window as unknown as { __clip: string[] }).__clip);

async function openRow(page: Page, i = 0) {
  await page.getByTestId('history-row').nth(i).locator('td.open').first().click();
  await expect(page.getByTestId('exchange-detail')).toBeVisible();
  await expect(page.getByTestId('detail-id')).toHaveText(ids[i]);
}

test('rail items reach every view, including Decoder and Settings', async ({ page }) => {
  await mock(page);
  await page.goto('/');
  for (const [nav, path, view] of [
    ['nav-decoder', '/decoder', 'decoder-pane'],
    ['nav-settings', '/settings', 'settings-view'],
    ['nav-repeater', '/repeater', 'repeater-view'],
    ['nav-intercept', '/intercept', 'intercept-view'],
    ['nav-scope', '/scope', 'scope-view'],
    ['nav-history', '/', 'history-view'],
  ] as const) {
    await page.getByTestId(nav).click();
    await expect(page).toHaveURL(new RegExp(`${path.replace('/', '\\/')}$`));
    await expect(page.getByTestId(view)).toBeVisible();
  }
});

test('selection toolbar Send to Repeater opens a tab and lands on /repeater', async ({ page }) => {
  await mock(page);
  await page.goto('/');
  await openRow(page);
  const send = page.getByTestId('selection-toolbar').getByTestId('send-to-repeater');
  await expect(send).toBeVisible();
  await send.click();
  await expect(page).toHaveURL(/\/repeater$/);
  await expect(page.getByTestId('repeater-tab')).toHaveCount(1);
  await expect(page.getByTestId('repeater-request').locator('.cm-content')).toContainText('GET /p/1?q=1 HTTP/1.1');
});

test('row context menu Copy URL writes the recorded URL', async ({ page }) => {
  await clipboardStub(page);
  await mock(page);
  await page.goto('/');
  await page.getByTestId('history-row').nth(1).click({ button: 'right' });
  const menu = page.getByTestId('row-menu');
  await expect(menu).toBeVisible();
  await menu.getByTestId('copy-url').click();
  // tls_version on the recorded connection makes it https; 443 is elided.
  await expect.poll(() => clip(page)).toEqual(['https://shop.example/p/2?q=1']);
  await expect(page.getByTestId('row-menu')).toHaveCount(0);
});

test('Decode selection in the History detail seeds /decoder', async ({ page }) => {
  await mock(page);
  await page.goto('/');
  await openRow(page);
  const body = page.getByTestId('detail-resp-body');
  await expect(body).toContainText(BODY);
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
  await expect(page.getByTestId('decoder-input')).toHaveValue(BODY);
  // The seed is consumed once: leaving and re-entering shows an empty pane.
  await page.getByTestId('nav-history').click();
  await page.getByTestId('nav-decoder').click();
  await expect(page.getByTestId('decoder-input')).toHaveValue('');
});

test('? opens the shortcuts dialog', async ({ page }) => {
  await mock(page);
  await page.goto('/');
  await expect(page.getByTestId('history-row')).toHaveCount(3);
  await page.getByTestId('nav-history').focus();
  await page.keyboard.press('?');
  await expect(page.getByTestId('shortcuts-dialog')).toBeVisible();
});

test('the theme toggle persists across a reload', async ({ page }) => {
  await mock(page);
  await page.goto('/');
  const html = page.locator('html');
  const before = await html.getAttribute('data-theme');
  await page.getByTestId('theme-toggle').click();
  const after = before === 'dark' ? 'light' : 'dark';
  await expect(html).toHaveAttribute('data-theme', after);
  await page.reload();
  await expect(html).toHaveAttribute('data-theme', after);
});

test('narrow detail pane: Request/Response switch shows one side at a time', async ({ page }) => {
  await mock(page);
  await page.goto('/');
  await openRow(page);
  // Default viewport (1280): the detail pane is under the side-by-side width.
  await expect(page.getByTestId('detail-side-resp')).toBeVisible();
  await expect(page.getByTestId('detail-response')).toBeVisible();
  await expect(page.getByTestId('detail-request')).toHaveCount(0);
  await page.getByTestId('detail-side-req').click();
  await expect(page.getByTestId('detail-request')).toBeVisible();
  await expect(page.getByTestId('detail-response')).toHaveCount(0);
  await page.getByTestId('detail-side-resp').click();
  await expect(page.getByTestId('detail-response')).toBeVisible();
  await expect(page.getByTestId('detail-request')).toHaveCount(0);
});

test('a lone CR stays inside one header row and shows as ␍', async ({ page }) => {
  await mock(page, {
    reqHead: 'GET / HTTP/1.1\r\nHost: shop.example\r\nX-A: one\rX-Smuggle: two\r\n\r\n',
  });
  await page.goto('/');
  await openRow(page);
  await page.getByTestId('detail-side-req').click();
  await page.getByTestId('detail-request').getByTestId('detail-tab-headers').click();
  const table = page.getByTestId('detail-req-headers');
  await expect(table).toBeVisible();
  const rows = table.locator('tbody tr');
  await expect(rows).toHaveCount(2);
  await expect(rows.nth(1).locator('th')).toHaveText('X-A');
  await expect(rows.nth(1).locator('td')).toHaveText('one␍X-Smuggle: two');
  await expect(table.locator('th', { hasText: /^X-Smuggle$/ })).toHaveCount(0);
});

test('the toolbar names its target when focus is on another row', async ({ page }) => {
  await clipboardStub(page);
  await mock(page);
  await page.goto('/');
  await openRow(page, 0);
  const bar = page.getByTestId('selection-toolbar');
  await expect(bar.getByTestId('selection-target')).toHaveCount(0);
  // Focus row 3 without opening it: toolbar and Ctrl+R now act on row 3.
  await page.getByTestId('history-row').nth(2).focus();
  const target = bar.getByTestId('selection-target');
  await expect(target).toBeVisible();
  await expect(target).toContainText('acting on #3');
  await expect(target).toContainText('/p/3?q=1');
  await expect(page.getByTestId('detail-id')).toHaveText(ids[0]);
  // Its Copy goes to the targeted row, not the open one.
  await bar.getByTestId('selection-copy').click();
  await page.getByTestId('copy-url').click();
  await expect.poll(() => clip(page)).toEqual(['https://shop.example/p/3?q=1']);
  // Focus back in the detail re-targets the open row.
  await page.getByTestId('notes-input').focus();
  await expect(bar.getByTestId('selection-target')).toHaveCount(0);
});

test('side by side, the detail-tab ids are scoped per message', async ({ page }) => {
  await page.setViewportSize({ width: 1700, height: 900 });
  // Give the detail most of the width so it crosses the side-by-side threshold.
  await page.addInitScript(() => localStorage.setItem('crosswire.split.history-detail', '0.35'));
  await mock(page);
  await page.goto('/');
  await openRow(page);
  const req = page.getByTestId('detail-request');
  const resp = page.getByTestId('detail-response');
  await expect(req).toBeVisible();
  await expect(resp).toBeVisible();
  await expect(page.getByTestId('detail-side-req')).toHaveCount(0);
  await expect(page.getByTestId('detail-tab-headers')).toHaveCount(2);
  await expect(req.getByTestId('detail-tab-headers')).toHaveCount(1);
  await expect(resp.getByTestId('detail-tab-headers')).toHaveCount(1);
  // Switching the request's tab leaves the response's tab alone.
  await req.getByTestId('detail-tab-headers').click();
  await expect(page.getByTestId('detail-req-headers')).toBeVisible();
  await expect(page.getByTestId('detail-resp-panel-raw')).toBeVisible();
  await expect(page.getByTestId('detail-resp-headers')).toHaveCount(0);
  await resp.getByTestId('detail-tab-hex').click();
  await expect(page.getByTestId('detail-resp-hex')).toBeVisible();
  await expect(page.getByTestId('detail-req-headers')).toBeVisible();
});
