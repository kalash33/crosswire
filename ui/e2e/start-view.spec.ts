import { test, expect } from '@playwright/test';
import { mockApp, mockEvents, row, type Row } from './fixtures/app-mock';

// U1 D3 Start view: an empty, unfiltered project shows setup guidance instead
// of an empty table; the first captured exchange replaces it.

test('an empty project shows the Start panel with the three setup steps', async ({ page }) => {
  await mockApp(page);
  await mockEvents(page);
  await page.goto('/');
  const panel = page.getByTestId('start-panel');
  await expect(panel).toBeVisible();
  await expect(panel).toContainText('127.0.0.1:8080');
  await expect(panel).toContainText('ca.pem');
  await expect(panel).toContainText('ca.der');
  await expect(page.getByTestId('start-browser-command')).toHaveText(
    'crosswire browser --project PROJECT_DIR --proxy 127.0.0.1:8080',
  );
  await expect(panel).toContainText('Replace PROJECT_DIR with your project directory.');
  // No href anywhere in the panel: the CA page opens via a button handler.
  await expect(panel.locator('a[href]')).toHaveCount(0);
  // Nothing to filter yet: the filter/bulk bar is hidden while the panel shows.
  await expect(page.getByTestId('filter-toggle')).toHaveCount(0);
  await expect(page.getByTestId('history-filters')).toHaveCount(0);
});

test('Open CA page opens the proxy CA page in a new tab', async ({ page }) => {
  await mockApp(page);
  await mockEvents(page);
  await page.addInitScript(() => {
    (window as unknown as { __opened: unknown[] }).__opened = [];
    window.open = ((...args: unknown[]) => {
      (window as unknown as { __opened: unknown[] }).__opened.push(args);
      return null;
    }) as typeof window.open;
  });
  await page.goto('/');
  await page.getByTestId('start-open-ca').click();
  const opened = await page.evaluate(() => (window as unknown as { __opened: unknown[] }).__opened);
  expect(opened).toEqual([['http://127.0.0.1:8080/', '_blank', 'noopener']]);
});

test('the first received event hides the Start panel', async ({ page }) => {
  await mockApp(page);
  const sockets = await mockEvents(page);
  await page.goto('/');
  await expect(page.getByTestId('start-panel')).toBeVisible();
  await expect(page.getByTestId('ws-dot')).toHaveAttribute('data-state', 'connected');
  sockets[0].send(JSON.stringify({ type: 'received', id: '01HSTARTSTARTSTARTSTARTSTA', seq: 1 }));
  await expect(page.getByTestId('start-panel')).toHaveCount(0);
  await expect(page.getByTestId('history-row')).toHaveCount(1);
});

test('rows arriving on a refresh hide the Start panel', async ({ page }) => {
  let rows: Row[] = [];
  await mockApp(page, { rows: () => rows });
  const sockets = await mockEvents(page);
  await page.goto('/');
  await expect(page.getByTestId('start-panel')).toBeVisible();
  rows = [row('01HROWROWROWROWROWROWROWROW')];
  // A lagged frame makes the store refetch the first page.
  sockets[0].send(JSON.stringify({ type: 'lagged', missed: 1, seq: 2 }));
  await expect(page.getByTestId('history-row')).toHaveCount(1);
  await expect(page.getByTestId('start-panel')).toHaveCount(0);
});

test('a filter with no matches shows the empty table, not the Start panel', async ({ page }) => {
  // The filter bar is hidden on the Start panel (U1), so start from a project
  // with a row; the filtered request then matches nothing.
  let rows: Row[] = [row('01HFILTERFILTERFILTERFILTE')];
  await mockApp(page, { rows: () => rows });
  await mockEvents(page);
  await page.goto('/');
  await expect(page.getByTestId('history-row')).toHaveCount(1);
  await expect(page.getByTestId('start-panel')).toHaveCount(0);
  rows = [];
  await page.getByTestId('filter-host').fill('nothing.example');
  await page.getByTestId('filter-apply').click();
  await expect(page.getByTestId('start-panel')).toHaveCount(0);
  await expect(page.getByTestId('history-empty')).toBeVisible();
});
