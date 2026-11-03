import { defineConfig, devices } from '@playwright/test';

// The history-view frame-time / filter-latency trace (§13), run against a REAL
// `crosswire serve` on a seeded synthetic project. scripts/ui-perf.sh builds,
// seeds and starts serve, then runs this config. Kept out of the default suite
// (playwright.config.ts testIgnores the spec), like first-capture-ui.
const API_URL = process.env.CROSSWIRE_API_URL;
if (!API_URL) {
  throw new Error('CROSSWIRE_API_URL is unset; run this config via `just ui-perf` / scripts/ui-perf.sh');
}
// The session saved by the global setup (the single-use bootstrap token is
// exchanged once there, so a retry starts already signed in).
const STATE = process.env.CROSSWIRE_UI_PERF_STATE;
if (!STATE) {
  throw new Error('CROSSWIRE_UI_PERF_STATE is unset; run this config via `just ui-perf` / scripts/ui-perf.sh');
}

export default defineConfig({
  testDir: './e2e',
  testMatch: /perf-scroll\.spec\.ts$/,
  fullyParallel: false,
  forbidOnly: !!process.env.CI,
  globalSetup: './e2e/perf-scroll.setup.ts',
  // One retry in CI absorbs a noisy shared runner: a fresh page on the same
  // served project and session re-runs the whole measurement. Playwright
  // reports a retried pass as "flaky"; it is a signal to read, not to ignore.
  retries: process.env.CI ? 1 : 0,
  workers: 1,
  // `list` prints the SUMMARY line (test stdout) so CI logs show the headroom;
  // `github` adds failure annotations in CI.
  reporter: process.env.CI ? [['list'], ['github']] : 'list',
  timeout: 180_000,
  // No trace, video or screenshot: they would capture the session cookie.
  use: { baseURL: API_URL, storageState: STATE, trace: 'off', video: 'off', screenshot: 'off' },
  projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'] } }],
});
