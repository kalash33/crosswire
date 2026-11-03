import { test, expect, type Page } from '@playwright/test';
import { mockApp, mockEvents, row, RULES } from './fixtures/app-mock';

// V1 scoped keymap: the viewer's Find is a keymap action that shares Ctrl+F
// with Intercept's Forward, conflicts are per scope, and Settings and the `?`
// sheet group shortcuts by scope.

const ROW_ID = '01HZZZZZZZZZZZZZZZZZZZZKEY2';
const HELD_ID = '01HZZZZZZZZZZZZZZZZZZZZHELD';
const HELD_HEAD = 'POST /h HTTP/1.1\r\nHost: held.example\r\nContent-Length: 2\r\n\r\n';
const b64 = (s: string) => Buffer.from(s, 'latin1').toString('base64');

interface Harness {
  forwards: string[];
}

/** mockApp plus one openable exchange and one held request (counted forwards). */
async function mockKeys(page: Page): Promise<Harness> {
  const h: Harness = { forwards: [] };
  await mockApp(page, { rows: () => [row(ROW_ID)] });
  await mockEvents(page);
  const msg = (head: string) => ({
    kind: 'http1-raw', fidelity: 'raw',
    head: { type: 'http1-raw', bytes: b64(head), parser_version: 1 },
    diagnostics: [], body: { type: 'none' },
  });
  await page.route(/\/api\/v1\/exchanges\/[^/?]+$/, (r) =>
    r.fulfill({
      status: 200,
      json: {
        id: ROW_ID,
        request: msg('GET / HTTP/1.1\r\nHost: a.example\r\n\r\n'),
        response: msg('HTTP/1.1 200 OK\r\n\r\n'),
        scope_state: 'in', state: 'committed', tags: [], notes: '',
      },
    }),
  );
  await page.route(/\/api\/v1\/exchanges\/[^/]+\/body\/(req|resp)$/, (r) =>
    r.fulfill({ status: 404, json: { error: { code: 'not_found', message: 'no body' } } }),
  );
  await page.route(/\/api\/v1\/repeater\/tabs\/[^/]+$/, (r) => r.fulfill({ status: 200, body: '' }));
  await page.route(/\/api\/v1\/intercept$/, (r) =>
    r.fulfill({
      status: 200,
      json: {
        enabled: true,
        rules: RULES,
        held: [{ id: HELD_ID, host: 'held.example', method: 'POST', path: '/h', size: 60, editable: true }],
      },
    }),
  );
  await page.route(/\/api\/v1\/intercept\/held\/[^/]+\/forward$/, (r) => {
    h.forwards.push(r.request().url().split('/').slice(-2)[0]);
    return r.fulfill({ status: 200, body: '' });
  });
  await page.route(/\/api\/v1\/intercept\/held\/[^/]+\/head$/, (r) =>
    r.fulfill({ status: 200, contentType: 'application/octet-stream', body: Buffer.from(HELD_HEAD, 'latin1') }),
  );
  await page.route(/\/api\/v1\/intercept\/held\/[^/]+\/body$/, (r) =>
    r.fulfill({ status: 200, contentType: 'application/octet-stream', body: Buffer.from('ok', 'latin1') }),
  );
  return h;
}

const shortcutRow = (page: Page, action: string) =>
  page.getByTestId('settings-shortcuts').locator('tr', { has: page.getByTestId(`shortcut-change-${action}`) });

async function openResponse(page: Page) {
  await page.getByTestId('history-row').locator('td.open').first().click();
  await expect(page.getByTestId('detail-id')).toHaveText(ROW_ID);
  const side = page.getByTestId('detail-side-resp');
  if ((await side.count()) > 0) await side.click();
  return page.getByTestId('detail-response');
}

test('defaults: Ctrl+F finds in the viewer and forwards in Intercept', async ({ page }) => {
  const h = await mockKeys(page);
  await page.goto('/');
  const resp = await openResponse(page);
  await resp.getByTestId('detail-tab-raw').click();
  await resp.getByTestId('detail-viewer').focus();
  await page.keyboard.press('Control+KeyF');
  await expect(resp.getByTestId('detail-findbar-input')).toBeFocused();
  expect(h.forwards).toHaveLength(0);

  await page.getByTestId('nav-intercept').click();
  await expect(page.getByTestId('intercept-view')).toBeVisible();
  await expect(page.getByTestId('held-row')).toHaveCount(1);
  await page.getByTestId('held-row').first().focus();
  await expect(page.locator('.cm-content').first()).toContainText('POST /h');
  await page.keyboard.press('Control+KeyF');
  await expect.poll(() => h.forwards.length).toBe(1);
});

test('remapping Find updates the viewer hint and the new chord opens find', async ({ page }) => {
  await mockKeys(page);
  await page.goto('/settings');
  await page.getByTestId('shortcut-change-find').click();
  await page.keyboard.press('Control+Shift+KeyF');
  await expect(shortcutRow(page, 'find').locator('kbd')).toHaveText('Ctrl+Shift+F');
  await expect(page.getByTestId('shortcut-error-find')).toHaveCount(0);

  await page.getByTestId('nav-history').click();
  const resp = await openResponse(page);
  await expect(resp.getByTestId('detail-find')).toHaveAttribute(
    'title',
    'Find in message (Ctrl+Shift+F in the viewer)',
  );
  await resp.getByTestId('detail-tab-raw').click();
  await resp.getByTestId('detail-viewer').focus();
  await page.keyboard.press('Control+Shift+KeyF');
  await expect(resp.getByTestId('detail-findbar-input')).toBeFocused();
});

test('conflicts are per scope: a global chord clashes with Forward, disjoint scopes share', async ({ page }) => {
  await mockKeys(page);
  await page.goto('/settings');
  await page.getByTestId('shortcut-change-copy-raw').click();
  await page.keyboard.press('Control+KeyF');
  await expect(page.getByTestId('shortcut-error-copy-raw')).toHaveText(
    'Ctrl+F is already used by "Forward the held request (Intercept)" (Intercept).',
  );
  await page.keyboard.press('Escape');
  await expect(shortcutRow(page, 'copy-raw').locator('kbd')).toHaveText('Ctrl+Shift+S');

  // Drop is Intercept's; Send is the Repeater's: the scopes never overlap.
  await page.getByTestId('shortcut-change-send').click();
  await page.keyboard.press('Control+KeyD');
  await expect(shortcutRow(page, 'send').locator('kbd')).toHaveText('Ctrl+D');
  await expect(page.getByTestId('shortcut-error-send')).toHaveCount(0);
  await expect(shortcutRow(page, 'drop').locator('kbd')).toHaveText('Ctrl+D');
});

test('hints follow a remap: top-bar switch and the selection toolbar Send', async ({ page }) => {
  await mockKeys(page);
  await page.goto('/settings');
  const sw = page.getByTestId('topbar-intercept');
  await expect(sw).toHaveAttribute('title', 'Hold requests at the proxy (Ctrl+I)');
  await page.getByTestId('shortcut-change-toggle-intercept').click();
  await page.keyboard.press('Control+Alt+KeyI');
  await expect(sw).toHaveAttribute('title', 'Hold requests at the proxy (Ctrl+Alt+I)');
  await page.getByTestId('shortcut-change-send-to-repeater').click();
  await page.keyboard.press('Control+Shift+KeyE');
  await expect(shortcutRow(page, 'send-to-repeater').locator('kbd')).toHaveText('Ctrl+Shift+E');

  await page.getByTestId('nav-history').click();
  await page.getByTestId('history-row').locator('td.open').first().click();
  await expect(page.getByTestId('detail-id')).toHaveText(ROW_ID);
  await expect(
    page.getByTestId('selection-toolbar').getByTestId('send-to-repeater').locator('kbd'),
  ).toHaveText('Ctrl+Shift+E');
});

test('Settings groups shortcuts by scope in order', async ({ page }) => {
  await mockKeys(page);
  await page.goto('/settings');
  const groups = page.getByTestId('settings-shortcuts').locator('section[data-testid^="shortcut-group-"]');
  await expect(groups.locator('h3')).toHaveText([
    'Everywhere',
    'History',
    'Repeater',
    'Intercept',
    'Message viewer',
  ]);
  const members: Record<string, string[]> = {
    global: ['send-to-repeater', 'toggle-intercept', 'decode-selection', 'copy-as-curl', 'copy-raw', 'show-shortcuts'],
    history: ['focus-search'],
    repeater: ['send', 'tab-next', 'tab-prev'],
    intercept: ['forward', 'drop'],
    viewer: ['find'],
  };
  for (const [scope, actions] of Object.entries(members)) {
    const g = page.getByTestId(`shortcut-group-${scope}`);
    for (const a of actions) await expect(g.getByTestId(`shortcut-change-${a}`)).toHaveCount(1);
  }
});

test('a stored U2 chord that Find now shadows is flagged on the Find row', async ({ page }) => {
  await page.addInitScript(() =>
    localStorage.setItem(
      'crosswire.keymap',
      JSON.stringify({ forward: 'ctrl+alt+f', 'toggle-intercept': 'ctrl+f' }),
    ),
  );
  await mockKeys(page);
  await page.goto('/settings');
  await expect(shortcutRow(page, 'find').locator('kbd')).toHaveText('Ctrl+F');
  await expect(page.getByTestId('shortcut-shadow-find')).toBeVisible();
  await expect(page.getByTestId('shortcut-shadow-forward')).toHaveCount(0);
});

test('the ? sheet groups shortcuts by scope and lists the History list keys', async ({ page }) => {
  await mockKeys(page);
  await page.goto('/');
  await page.getByTestId('nav-scope').focus();
  await page.keyboard.press('?');
  const dialog = page.getByTestId('shortcuts-dialog');
  await expect(dialog).toBeVisible();
  await expect(dialog.locator('h3')).toHaveText([
    'Everywhere',
    'History',
    'Repeater',
    'Intercept',
    'Message viewer',
    'History list',
  ]);
  await expect(dialog.locator('[data-testid="shortcut-group-viewer"] [data-action="find"]')).toContainText('Ctrl+F');
  const fixed = dialog.getByTestId('shortcut-fixed-row');
  await expect(fixed.first()).toBeVisible();
  await expect(dialog.getByTestId('shortcut-group-history-list')).toContainText('Page Down');
});

test('with request and response side by side, Ctrl+F finds only in the focused viewer', async ({ page }) => {
  await page.setViewportSize({ width: 1800, height: 900 });
  await mockKeys(page);
  await page.goto('/');
  await page.getByTestId('history-row').locator('td.open').first().click();
  await expect(page.getByTestId('detail-id')).toHaveText(ROW_ID);
  // The layout follows the detail pane's own width: require side-by-side mode.
  await expect.poll(async () => (await page.getByTestId('history-detail').boundingBox())?.width ?? 0).toBeGreaterThanOrEqual(760);
  const req = page.getByTestId('detail-request');
  const resp = page.getByTestId('detail-response');
  await expect(req).toBeVisible();
  await expect(resp).toBeVisible();
  await req.getByTestId('detail-tab-raw').click();
  await resp.getByTestId('detail-tab-raw').click();

  await req.getByTestId('detail-viewer').focus();
  await page.keyboard.press('Control+KeyF');
  await expect(req.getByTestId('detail-findbar-input')).toBeFocused();
  await expect(resp.getByTestId('detail-findbar-input')).toHaveCount(0);

  // Reverse, from a fresh page so neither find bar is open.
  await page.reload();
  await page.getByTestId('history-row').locator('td.open').first().click();
  await expect(page.getByTestId('detail-id')).toHaveText(ROW_ID);
  await req.getByTestId('detail-tab-raw').click();
  await resp.getByTestId('detail-tab-raw').click();
  await resp.getByTestId('detail-viewer').focus();
  await page.keyboard.press('Control+KeyF');
  await expect(resp.getByTestId('detail-findbar-input')).toBeFocused();
  await expect(req.getByTestId('detail-findbar-input')).toHaveCount(0);
});
