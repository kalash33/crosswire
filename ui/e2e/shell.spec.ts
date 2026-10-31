import { test, expect } from '@playwright/test';
import { mockApp, mockEvents } from './fixtures/app-mock';

// U1 D3 shell chrome: rail, top bar, shortcuts sheet, theme, live-socket dot.

test.beforeEach(async ({ page }) => {
  await mockApp(page);
});

test('every rail item navigates and marks itself current', async ({ page }) => {
  await mockEvents(page);
  await page.goto('/');
  const items: [string, string, string][] = [
    ['nav-repeater', '/repeater', 'repeater-view'],
    ['nav-intercept', '/intercept', 'intercept-view'],
    ['nav-scope', '/scope', 'scope-view'],
    ['nav-decoder', '/decoder', 'decoder-pane'],
    ['nav-settings', '/settings', 'settings-view'],
    ['nav-history', '/', 'history-view'],
  ];
  for (const [nav, path, view] of items) {
    await page.getByTestId(nav).click();
    await expect(page).toHaveURL(new RegExp(`${path.replace('/', '\\/')}$`));
    await expect(page.getByTestId(nav)).toHaveAttribute('aria-current', 'page');
    await expect(page.getByTestId(view)).toBeVisible();
    await expect(page.locator('[aria-current="page"]')).toHaveCount(1);
  }
});

test('? opens the shortcuts sheet; Escape closes it and restores focus', async ({ page }) => {
  await mockEvents(page);
  await page.goto('/');
  const anchor = page.getByTestId('nav-scope');
  await anchor.focus();
  await page.keyboard.press('?');
  const dialog = page.getByTestId('shortcuts-dialog');
  await expect(dialog).toBeVisible();
  await expect(dialog).toContainText('Send to Repeater');
  await expect(dialog).toContainText('Ctrl+Shift+D');
  await page.keyboard.press('Escape');
  await expect(dialog).toHaveCount(0);
  await expect(anchor).toBeFocused();
});

test('the top-bar Shortcuts button opens the sheet', async ({ page }) => {
  await mockEvents(page);
  await page.goto('/');
  await page.getByTestId('shortcuts-button').click();
  await expect(page.getByTestId('shortcuts-dialog')).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(page.getByTestId('shortcuts-dialog')).toHaveCount(0);
  await expect(page.getByTestId('shortcuts-button')).toBeFocused();
});

test('? typed into a text field stays text', async ({ page }) => {
  await mockEvents(page);
  await page.goto('/');
  const input = page.getByTestId('filter-host');
  await input.click();
  await page.keyboard.press('?');
  await expect(input).toHaveValue('?');
  await expect(page.getByTestId('shortcuts-dialog')).toHaveCount(0);
});

test('theme toggle flips data-theme and persists across reload', async ({ page }) => {
  await mockEvents(page);
  await page.goto('/');
  const html = page.locator('html');
  await expect(html).toHaveAttribute('data-theme', 'dark');
  await page.getByTestId('theme-toggle').click();
  await expect(html).toHaveAttribute('data-theme', 'light');
  await page.reload();
  await expect(page.getByTestId('app-shell')).toBeVisible();
  await expect(html).toHaveAttribute('data-theme', 'light');
  await page.getByTestId('theme-toggle').click();
  await expect(html).toHaveAttribute('data-theme', 'dark');
});

test('top-bar intercept switch mirrors the status-bar toggle', async ({ page }) => {
  await mockEvents(page);
  let enabled = false;
  await page.route(/\/api\/v1\/intercept$/, (r) =>
    r.fulfill({
      status: 200,
      json: { enabled, rules: { skip_extensions: [], skip_out_of_scope: false, overflow: 'forward-newest', on_timeout: 'forward' }, held: [] },
    }),
  );
  await page.route(/\/api\/v1\/intercept\/enabled$/, async (r) => {
    enabled = (r.request().postDataJSON() as { enabled: boolean }).enabled;
    await r.fulfill({ status: 204 });
  });
  await page.goto('/');
  const sw = page.getByTestId('topbar-intercept');
  await expect(sw).toHaveAttribute('aria-checked', 'false');
  await sw.click();
  await expect(sw).toHaveAttribute('aria-checked', 'true');
  await expect(page.getByTestId('status-intercept')).toContainText('Intercept ON');
  await page.getByTestId('intercept-toggle').click();
  await expect(sw).toHaveAttribute('aria-checked', 'false');
});

test('ws dot follows the socket; one toast per transition', async ({ page }) => {
  const sockets = await mockEvents(page);
  await page.goto('/');
  const dot = page.getByTestId('ws-dot');
  await expect(dot).toHaveAttribute('data-state', 'connected');
  await expect(page.getByTestId('status-ws')).toHaveAttribute('data-state', 'connected');

  // Drop: the page reconnects (backoff) to a fresh routed socket.
  await sockets[0].close();
  await expect(dot).toHaveAttribute('data-state', 'disconnected');
  await expect(page.getByTestId('toast').filter({ hasText: 'Live updates lost' })).toHaveCount(1);
  await expect(dot).toHaveAttribute('data-state', 'connected');
  await expect(page.getByTestId('toast').filter({ hasText: 'Live updates reconnected' })).toHaveCount(1);
  await expect(page.getByTestId('toast').filter({ hasText: 'Live updates lost' })).toHaveCount(1);
});

test('global chords do not fire while the shortcuts sheet is open', async ({ page }) => {
  await mockEvents(page);
  let puts = 0;
  await page.route(/\/api\/v1\/intercept\/enabled$/, async (r) => {
    puts += 1;
    await r.fulfill({ status: 204 });
  });
  await page.goto('/');
  await expect(page.getByTestId('topbar-intercept')).toBeEnabled();
  await page.getByTestId('shortcuts-button').click();
  await expect(page.getByTestId('shortcuts-dialog')).toBeVisible();
  await page.keyboard.press('Control+t');
  await page.keyboard.press('Control+Shift+D');
  await expect(page.getByTestId('shortcuts-dialog')).toBeVisible();
  await expect(page).toHaveURL(/\/$/);
  expect(puts).toBe(0);
  await expect(page.getByTestId('status-intercept')).toContainText('Intercept OFF');
});
