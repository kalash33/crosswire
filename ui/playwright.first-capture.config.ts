import { defineConfig, devices } from '@playwright/test';

// The install-to-first-capture UI gate (§12 DoD), run against a REAL
// `crosswire serve` — NOT vite preview, NOT mocked. scripts/first-capture-ui.sh
// builds and starts serve (embedded production bundle, ephemeral ports), then
// runs this config with the serve addresses + bootstrap token in the
// environment. Everything the spec needs is injected; this config starts no
// server of its own.
//
// Kept separate from playwright.config.ts (the vite-preview suite) so neither
// bleeds into the other: that config `testIgnore`s this spec, and this config
// runs ONLY it.
const API_URL = process.env.CROSSWIRE_API_URL;
if (!API_URL) {
  throw new Error(
    'CROSSWIRE_API_URL is unset; run this config via `just first-capture-ui` / scripts/first-capture-ui.sh',
  );
}

export default defineConfig({
  testDir: './e2e',
  testMatch: /first-capture-ui\.spec\.ts$/,
  fullyParallel: false,
  forbidOnly: !!process.env.CI,
  // One real capture; a flake here means the gate is broken, so don't paper
  // over it with retries.
  retries: 0,
  workers: 1,
  reporter: process.env.CI ? 'github' : 'list',
  // Wall-clock room for serve startup + the capture; the spec enforces the
  // 120 s install-to-first-capture budget itself.
  timeout: 200_000,
  use: {
    baseURL: API_URL,
    trace: 'on-first-retry',
  },
  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'] },
    },
  ],
  // No webServer: serve is already running (started by the script).
});
