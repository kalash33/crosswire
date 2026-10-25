import { test, expect } from '@playwright/test';
import { spawnSync } from 'node:child_process';

// Install-to-first-capture, THROUGH THE REAL UI (§12 DoD). Unlike the other
// e2e specs, nothing here is mocked: `scripts/first-capture-ui.sh` has already
// built and started a real `crosswire serve` (embedded production bundle,
// ephemeral ports), and handed us its addresses + a single-use bootstrap token
// via the environment. We drive the real served SPA — exercising the server's
// per-response CSP nonce, the real cookie auth, and the live events WS that the
// vite-preview specs can't — cause ONE real capture through the proxy, and
// assert the example.com exchange appears in the history view under the budget.
//
// This spec is excluded from the default `npm run e2e` (testIgnore in
// playwright.config.ts) and runs only under playwright.first-capture.config.ts.

const API = requireEnv('CROSSWIRE_API_URL'); // http://127.0.0.1:<apiPort>
const TOKEN = requireEnv('CROSSWIRE_BOOTSTRAP_TOKEN');
const PROXY = requireEnv('CROSSWIRE_PROXY'); // 127.0.0.1:<proxyPort>
const BIN = requireEnv('CROSSWIRE_BIN');
const PROJECT = requireEnv('CROSSWIRE_PROJECT');
const CA = requireEnv('CROSSWIRE_CA');
// The clock starts at serve launch (the script's START), excluding the build —
// exactly like scripts/first-capture.sh.
const START_MS = Number(requireEnv('CROSSWIRE_START')) * 1000;
const LIMIT_MS = 120_000;

function requireEnv(name: string): string {
  const v = process.env[name];
  if (!v) throw new Error(`missing env ${name} (run via scripts/first-capture-ui.sh)`);
  return v;
}

test('install-to-first-capture appears in the real UI history under 2 min', async ({ page }) => {
  // Budget covers serve startup + UI load + the capture + its appearance; give
  // Playwright room beyond the 120 s gate so a miss fails as an assertion.
  test.setTimeout(LIMIT_MS + 60_000);

  // 1. Open the REAL served SPA with the bootstrap fragment. The SPA exchanges
  // the single-use token for the HttpOnly session cookie, then renders the app.
  await page.goto(`${API}/#bootstrap=${TOKEN}`);
  await expect(page.getByTestId('app-shell')).toBeVisible();
  await expect(page.getByTestId('history-view')).toBeVisible();
  // The token must not linger in the address bar after the exchange.
  expect(page.url()).not.toContain('bootstrap');

  // 2. Cause ONE real capture through the running proxy. serve holds the
  // project's exclusive lock, so the launcher is pointed at the proxy with
  // --proxy. Real network egress to https://example.com/ happens here.
  // CROSSWIRE_BROWSER is inherited from the script's environment.
  const launch = spawnSync(
    BIN,
    [
      'browser',
      '--project', PROJECT,
      '--ca-dir', CA,
      '--proxy', PROXY,
      '--headless',
      '--url', 'https://example.com/',
      '--',
      '--no-sandbox',
      '--disable-gpu',
      '--disable-dev-shm-usage',
      '--disable-extensions',
      '--disable-background-networking',
      '--disable-component-update',
      '--disable-sync',
      '--disable-default-apps',
      '--dump-dom',
      '--timeout=15000',
    ],
    { encoding: 'utf8', timeout: 60_000 },
  );
  expect(launch.status, `crosswire browser failed:\n${launch.stderr}\n${launch.stdout}`).toBe(0);

  // 3. The example.com exchange must show in the history VIEW. A live
  // `received` event prepends a host-less placeholder, so we narrow by
  // host_contains — that refetch returns the committed row with its real host.
  // Retry the refetch (there can be a brief lag between the browser exiting and
  // the exchange being committed/indexed) until the row is on screen.
  await page.getByTestId('filter-host-contains').fill('example.com');
  const exampleRow = page
    .getByTestId('history-row')
    .filter({ hasText: 'example.com' });
  await expect(async () => {
    await page.getByTestId('filter-apply').click();
    await expect(exampleRow.first()).toBeVisible({ timeout: 2_000 });
  }).toPass({ timeout: LIMIT_MS, intervals: [500] });

  const elapsedMs = Date.now() - START_MS;
  // eslint-disable-next-line no-console
  console.log(`first capture visible in the UI history in ${Math.round(elapsedMs / 1000)}s (limit 120s)`);
  expect(elapsedMs, 'install-to-first-capture exceeded the 120 s budget').toBeLessThan(LIMIT_MS);
});
