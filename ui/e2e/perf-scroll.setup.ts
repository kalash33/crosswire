import { request } from '@playwright/test';
import { chmodSync, writeFileSync } from 'node:fs';

// Global setup for playwright.perf-scroll.config.ts. The bootstrap token is
// single-use with a 60 s TTL, so it cannot be put in each attempt's URL: a
// retry would present a spent token and land on the Auth screen. Exchange it
// ONCE here (POST /api/v1/session) and save the resulting session as a
// Playwright storageState file that every attempt loads.
//
// The file holds the HttpOnly session cookie. scripts/ui-perf.sh points
// CROSSWIRE_UI_PERF_STATE into its private `mktemp -d` directory (removed by
// its EXIT trap); it is written 0600 and its contents are never printed.
export default async function globalSetup(): Promise<void> {
  const api = need('CROSSWIRE_API_URL');
  const token = need('CROSSWIRE_BOOTSTRAP_TOKEN');
  const statePath = need('CROSSWIRE_UI_PERF_STATE');
  const ctx = await request.newContext({ baseURL: api });
  try {
    const res = await ctx.post('/api/v1/session', { data: { bootstrap: token } });
    // Status only: the response carries the Set-Cookie header.
    if (!res.ok()) throw new Error(`bootstrap exchange failed: HTTP ${res.status()}`);
    const state = await ctx.storageState();
    writeFileSync(statePath, JSON.stringify(state), { mode: 0o600 });
    chmodSync(statePath, 0o600);
  } finally {
    await ctx.dispose();
  }
}

function need(name: string): string {
  const v = process.env[name];
  if (!v) throw new Error(`missing env ${name} (run via scripts/ui-perf.sh)`);
  return v;
}
