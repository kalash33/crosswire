import { test, expect } from '@playwright/test';
import { mockApp, mockEvents } from './fixtures/app-mock';

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
  await expect(card).toContainText('Ctrl+T');
});
