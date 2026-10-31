import { test, expect, type Page } from '@playwright/test';

// U1 D3 shell contract: the app is a fixed-height grid. At laptop and desktop
// sizes, on every primary route, the status bar sits fully inside the viewport
// and the DOCUMENT never scrolls (only the main region scrolls internally).
//
// No backend: every API call the views make on mount is mocked.

const COUNTS = {
  bytes_inline: 0, bytes_packs: 0, committed: 0, error: 0, exchanges: 0,
  history: 0, intruder: 0, markers: 0, repeater: 0,
};
const RULES = {
  skip_extensions: [] as string[],
  skip_out_of_scope: false,
  overflow: 'forward-newest',
  on_timeout: 'forward',
};

async function mockApi(page: Page): Promise<void> {
  await page.route('**/api/v1/counts', (r) => r.fulfill({ status: 200, json: COUNTS }));
  await page.route('**/api/v1/scope', (r) =>
    r.fulfill({ status: 200, json: { include: [], exclude: [] } }),
  );
  await page.route(/\/api\/v1\/repeater\/tabs$/, (r) => r.fulfill({ status: 200, json: [] }));
  await page.route(/\/api\/v1\/intercept$/, (r) =>
    r.fulfill({ status: 200, json: { enabled: false, rules: RULES, held: [] } }),
  );
  await page.route(/\/api\/v1\/exchanges(\?.*)?$/, (r) =>
    r.fulfill({ status: 200, json: { items: [], next_cursor: null } }),
  );
}

const VIEWPORTS = [
  { width: 1024, height: 700 },
  { width: 1440, height: 900 },
];
const ROUTES = ['/', '/repeater', '/intercept', '/scope', '/decoder', '/settings'];

for (const vp of VIEWPORTS) {
  for (const route of ROUTES) {
    test(`shell fits ${vp.width}x${vp.height} on ${route}`, async ({ page }) => {
      await page.setViewportSize(vp);
      await mockApi(page);
      await page.goto(route);
      await expect(page.getByTestId('app-shell')).toBeVisible();

      const bar = page.getByTestId('status-bar');
      await expect(bar).toBeVisible();
      const box = await bar.boundingBox();
      expect(box).not.toBeNull();
      expect(box!.x).toBeGreaterThanOrEqual(0);
      expect(box!.y).toBeGreaterThanOrEqual(0);
      expect(box!.x + box!.width).toBeLessThanOrEqual(vp.width);
      expect(box!.y + box!.height).toBeLessThanOrEqual(vp.height);

      const { scrollHeight, scrollWidth, innerHeight, innerWidth } = await page.evaluate(() => ({
        scrollHeight: document.scrollingElement!.scrollHeight,
        scrollWidth: document.scrollingElement!.scrollWidth,
        innerHeight: window.innerHeight,
        innerWidth: window.innerWidth,
      }));
      expect(scrollHeight).toBeLessThanOrEqual(innerHeight);
      expect(scrollWidth).toBeLessThanOrEqual(innerWidth);
    });
  }
}
