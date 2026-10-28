import { defineConfig, devices } from '@playwright/test';

// The history-view frame-time / filter-latency trace (§13), run against a REAL
// `crosswire serve` on a seeded synthetic project. scripts/ui-perf.sh builds,
// seeds and starts serve, then runs this config. Kept out of the default suite
// (playwright.config.ts testIgnores the spec), like first-capture-ui.
const API_URL = process.env.CROSSWIRE_API_URL;
if (!API_URL) {
  throw new Error('CROSSWIRE_API_URL is unset; run this config via `just ui-perf` / scripts/ui-perf.sh');
}

export default defineConfig({
  testDir: './e2e',
  testMatch: /perf-scroll\.spec\.ts$/,
  fullyParallel: false,
  forbidOnly: !!process.env.CI,
  retries: 0,
  workers: 1,
  reporter: process.env.CI ? 'github' : 'list',
  timeout: 180_000,
  use: { baseURL: API_URL },
  projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'] } }],
});
