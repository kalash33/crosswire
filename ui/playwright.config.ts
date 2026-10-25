import { defineConfig, devices } from '@playwright/test';

// Shared e2e harness. Later tasks add *.spec.ts gates under ui/e2e/.
//
// The app is a static SPA; its first paint makes no backend call, so e2e runs
// against `vite preview` serving the built bundle — no `crosswire serve`
// needed. Build first (`npm run build`), or let the webServer command do it.
// Override the port with PLAYWRIGHT_PORT.
const PORT = Number(process.env.PLAYWRIGHT_PORT ?? 4173);
const BASE_URL = `http://127.0.0.1:${PORT}`;

export default defineConfig({
  testDir: './e2e',
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  reporter: process.env.CI ? 'github' : 'list',
  use: {
    baseURL: BASE_URL,
    trace: 'on-first-retry',
  },
  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'] },
    },
  ],
  webServer: {
    command: `npm run build && npm run preview -- --host 127.0.0.1 --port ${PORT} --strictPort`,
    url: BASE_URL,
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
  },
});
