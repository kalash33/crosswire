import { test, expect, type Page } from '@playwright/test';
import { mockApp, mockEvents, row } from './fixtures/app-mock';

// U1 D6 Settings route: appearance, shortcuts, connection, about.

test('About shows the schema and format versions from /health', async ({ page }) => {
  await mockApp(page, { health: { status: 'ok', schema_version: 3, format_version: 7 } });
  await mockEvents(page);
  await page.goto('/settings');
  await expect(page.getByTestId('settings-view')).toBeVisible();
  await expect(page.getByTestId('about-schema-version')).toHaveText('3');
  await expect(page.getByTestId('about-format-version')).toHaveText('7');
  await expect(page.getByTestId('about-version')).toHaveCount(0);
  await expect(page.getByTestId('settings-connection')).toContainText('127.0.0.1');
});

test('Appearance switches the theme', async ({ page }) => {
  await mockApp(page);
  await mockEvents(page);
  await page.goto('/settings');
  await page.getByTestId('theme-light').click();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
  await expect(page.getByTestId('theme-light')).toHaveAttribute('aria-pressed', 'true');
  await page.getByTestId('theme-dark').click();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
});

test('Shortcuts card lists the keymap', async ({ page }) => {
  await mockApp(page);
  await mockEvents(page);
  await page.goto('/settings');
  const card = page.getByTestId('settings-shortcuts');
  await expect(card).toContainText('Toggle intercept');
  await expect(card).toContainText('Ctrl+I');
});

// ---- U2 D5: remappable shortcuts -------------------------------------------

const ROW_ID = '01HZZZZZZZZZZZZZZZZZZZZKEY1';
const b64 = (s: string) => Buffer.from(s, 'latin1').toString('base64');

/** mockApp plus one openable exchange and the intercept switch (counted). */
async function mockKeys(page: Page): Promise<{ interceptPuts: number }> {
  const h = { interceptPuts: 0 };
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
  await page.route(/\/api\/v1\/intercept\/enabled$/, (r) => {
    h.interceptPuts++;
    return r.fulfill({ status: 200, body: '' });
  });
  return h;
}

const shortcutRow = (page: Page, action: string) =>
  page.getByTestId('settings-shortcuts').locator('tr', { has: page.getByTestId(`shortcut-change-${action}`) });

test('remap Send to Repeater to Ctrl+Shift+E: badge, persisted, hint updated, Ctrl+R inert', async ({ page }) => {
  await mockKeys(page);
  await page.goto('/settings');
  const tr = shortcutRow(page, 'send-to-repeater');
  await expect(tr.locator('kbd')).toHaveText('Ctrl+R');
  await page.getByTestId('shortcut-change-send-to-repeater').click();
  await expect(page.getByTestId('shortcut-change-send-to-repeater')).toHaveText('Recording…');
  await page.keyboard.press('Control+Shift+KeyE');
  await expect(tr.locator('kbd')).toHaveText('Ctrl+Shift+E');
  await expect(tr).toContainText('custom');
  await expect(page.getByTestId('shortcut-change-send-to-repeater')).toHaveText('Change');
  expect(await page.evaluate(() => localStorage.getItem('crosswire.keymap'))).toContain('ctrl+shift+e');

  await page.reload();
  await expect(shortcutRow(page, 'send-to-repeater').locator('kbd')).toHaveText('Ctrl+Shift+E');
  await expect(shortcutRow(page, 'send-to-repeater')).toContainText('custom');

  // History: the selection toolbar hint names the new chord; Ctrl+R does nothing.
  await page.getByTestId('nav-history').click();
  await page.getByTestId('history-row').locator('td.open').first().click();
  await expect(page.getByTestId('detail-id')).toHaveText(ROW_ID);
  const send = page.getByTestId('selection-toolbar').getByTestId('send-to-repeater');
  await expect(send.locator('kbd')).toHaveText('Ctrl+Shift+E');
  await page.getByTestId('history-row').first().focus();
  await page.keyboard.press('Control+KeyR');
  await page.waitForTimeout(300);
  await expect(page).not.toHaveURL(/\/repeater$/);
  await expect(page.getByTestId('history-view')).toBeVisible();
  // The new chord sends the row to the Repeater.
  await page.getByTestId('history-row').first().focus();
  await page.keyboard.press('Control+Shift+KeyE');
  await expect(page).toHaveURL(/\/repeater$/);
  await expect(page.getByTestId('repeater-tab')).toHaveCount(1);
});

test('a reserved chord is refused with a message; Esc cancels recording', async ({ page }) => {
  await mockKeys(page);
  await page.goto('/settings');
  const btn = page.getByTestId('shortcut-change-send-to-repeater');
  await btn.click();
  await page.keyboard.press('Control+KeyT');
  await expect(page.getByTestId('shortcut-error-send-to-repeater')).toContainText('reserved');
  await expect(btn).toHaveText('Recording…');
  await page.keyboard.press('Escape');
  await expect(btn).toHaveText('Change');
  await expect(page.getByTestId('shortcut-error-send-to-repeater')).toHaveCount(0);
  await expect(shortcutRow(page, 'send-to-repeater').locator('kbd')).toHaveText('Ctrl+R');
  expect(await page.evaluate(() => localStorage.getItem('crosswire.keymap'))).toBeNull();
});

test('a bare Tab is refused while recording and keeps focus on the button', async ({ page }) => {
  await mockKeys(page);
  await page.goto('/settings');
  const btn = page.getByTestId('shortcut-change-copy-raw');
  await btn.click();
  await page.keyboard.press('Tab');
  await expect(page.getByTestId('shortcut-error-copy-raw')).toBeVisible();
  await expect(btn).toBeFocused();
  await expect(shortcutRow(page, 'copy-raw').locator('kbd')).toHaveText('Ctrl+Shift+S');
  // A chord held by another action is refused naming it.
  await page.keyboard.press('Control+KeyR');
  await expect(page.getByTestId('shortcut-error-copy-raw')).toContainText('Send to Repeater');
  await page.keyboard.press('Escape');
  await expect(shortcutRow(page, 'copy-raw').locator('kbd')).toHaveText('Ctrl+Shift+S');
});

test('recording a chord does not also trigger its action', async ({ page }) => {
  const h = await mockKeys(page);
  await page.goto('/settings');
  // Toggle intercept is a global chord: recording its new chord must not fire it.
  await page.getByTestId('shortcut-change-toggle-intercept').click();
  await page.keyboard.press('Control+Alt+KeyI');
  await expect(shortcutRow(page, 'toggle-intercept').locator('kbd')).toHaveText('Ctrl+Alt+I');
  await page.waitForTimeout(300);
  expect(h.interceptPuts).toBe(0);
  // Re-recording the action's own current chord does not fire it either.
  await page.getByTestId('shortcut-change-toggle-intercept').click();
  await page.keyboard.press('Control+Alt+KeyI');
  await page.waitForTimeout(300);
  expect(h.interceptPuts).toBe(0);
  // The help chord `?` is refused while recording and opens no sheet.
  await page.getByTestId('shortcut-change-toggle-intercept').click();
  await page.keyboard.press('?');
  await expect(page.getByTestId('shortcut-error-toggle-intercept')).toBeVisible();
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await page.keyboard.press('Escape');
  // Outside recording the new chord is live (so the checks above were meaningful).
  await page.getByTestId('settings-view').locator('h1').click();
  await page.keyboard.press('Control+Alt+KeyI');
  await expect.poll(() => h.interceptPuts).toBe(1);
});

test('Reset and Reset all restore the defaults', async ({ page }) => {
  await mockKeys(page);
  await page.goto('/settings');
  await page.getByTestId('shortcut-change-send-to-repeater').click();
  await page.keyboard.press('Control+Shift+KeyE');
  await page.getByTestId('shortcut-change-copy-raw').click();
  await page.keyboard.press('Control+Alt+KeyS');
  await expect(shortcutRow(page, 'copy-raw').locator('kbd')).toHaveText('Ctrl+Alt+S');

  await page.getByTestId('shortcut-reset-send-to-repeater').click();
  await expect(shortcutRow(page, 'send-to-repeater').locator('kbd')).toHaveText('Ctrl+R');
  await expect(shortcutRow(page, 'send-to-repeater')).not.toContainText('custom');
  await expect(page.getByTestId('shortcut-reset-send-to-repeater')).toHaveCount(0);
  await expect(shortcutRow(page, 'copy-raw').locator('kbd')).toHaveText('Ctrl+Alt+S');

  await page.getByTestId('shortcuts-reset-all').click();
  await expect(shortcutRow(page, 'copy-raw').locator('kbd')).toHaveText('Ctrl+Shift+S');
  await expect(page.getByTestId('shortcuts-reset-all')).toHaveCount(0);
  await page.reload();
  await expect(shortcutRow(page, 'send-to-repeater').locator('kbd')).toHaveText('Ctrl+R');
  await expect(shortcutRow(page, 'copy-raw').locator('kbd')).toHaveText('Ctrl+Shift+S');
});

test('the TopBar intercept hint follows a remap without a reload', async ({ page }) => {
  await mockKeys(page);
  await page.goto('/settings');
  const sw = page.getByTestId('topbar-intercept');
  await expect(sw).toHaveAttribute('title', 'Hold requests at the proxy (Ctrl+I)');
  await page.getByTestId('shortcut-change-toggle-intercept').click();
  await page.keyboard.press('Control+Alt+KeyI');
  await expect(shortcutRow(page, 'toggle-intercept').locator('kbd')).toHaveText('Ctrl+Alt+I');
  await expect(sw).toHaveAttribute('title', 'Hold requests at the proxy (Ctrl+Alt+I)');
  await page.getByTestId('shortcuts-reset-all').click();
  await expect(sw).toHaveAttribute('title', 'Hold requests at the proxy (Ctrl+I)');
});

test('Ctrl+F inside a viewer opens find and never reaches the global keymap', async ({ page }) => {
  // Bind Ctrl+F to a global action (toggle intercept) so a leak is observable
  // from any route; Forward moves out of the way.
  await page.addInitScript(() =>
    localStorage.setItem(
      'crosswire.keymap',
      JSON.stringify({ forward: 'ctrl+alt+f', 'toggle-intercept': 'ctrl+f' }),
    ),
  );
  const h = await mockKeys(page);
  await page.goto('/');
  await page.getByTestId('history-row').locator('td.open').first().click();
  await expect(page.getByTestId('detail-id')).toHaveText(ROW_ID);
  const side = page.getByTestId('detail-side-resp');
  if ((await side.count()) > 0) await side.click();
  const resp = page.getByTestId('detail-response');
  await expect(resp.getByTestId('detail-find')).toHaveAttribute('title', 'Find in message (Ctrl+F in the viewer)');
  // A textual tab: Ctrl+F opens the viewer's find bar.
  await resp.getByTestId('detail-tab-raw').click();
  await resp.getByTestId('detail-viewer').focus();
  await page.keyboard.press('Control+KeyF');
  await expect(resp.getByTestId('detail-findbar-input')).toBeFocused();
  // Headers (not textual): the browser keeps Ctrl+F, the app still never sees it.
  await resp.getByTestId('detail-tab-headers').click();
  await resp.getByTestId('detail-viewer').focus();
  await page.keyboard.press('Control+KeyF');
  await page.waitForTimeout(300);
  expect(h.interceptPuts).toBe(0);
  // Outside the viewer the chord is live (so the checks above were meaningful).
  await page.getByTestId('history-row').first().focus();
  await page.keyboard.press('Control+KeyF');
  await expect.poll(() => h.interceptPuts).toBe(1);
});
