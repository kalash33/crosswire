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
    // E2E=1 makes the build include the ByteEditor fixture entry (see
    // vite.config.ts). preview then serves it at /e2e/fixture/index.html. This
    // never touches the committed production bundle, which is built without E2E.
    command: `npm run build && npm run preview -- --host 127.0.0.1 --port ${PORT} --strictPort`,
    env: { E2E: '1' },
    url: BASE_URL,
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
  },
});
