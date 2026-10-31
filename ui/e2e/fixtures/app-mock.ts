import type { Page, WebSocketRoute } from '@playwright/test';

// A backend-free app fixture for the shell/start/settings specs: every API
// call the views make on mount is fulfilled, and the live events socket is
// served in-page by Playwright (no `crosswire serve`).

export const COUNTS = {
  bytes_inline: 0, bytes_packs: 0, committed: 0, error: 0, exchanges: 0,
  history: 0, intruder: 0, markers: 0, repeater: 0,
};
export const RULES = {
  skip_extensions: [] as string[],
  skip_out_of_scope: false,
  overflow: 'forward-newest',
  on_timeout: 'forward',
};
export const HEALTH = { status: 'ok', schema_version: 3, format_version: 7 };

export interface Row {
  id: string; created_at: number; method: string; host: string; path: string;
  status: number; resp_len: number; state: string; tags: string[];
}

export function row(id: string): Row {
  return {
    id, created_at: 1_700_000_000_000, method: 'GET', host: 'a.example', path: '/',
    status: 200, resp_len: 2, state: 'committed', tags: [],
  };
}

export interface MockOptions {
  /** Rows the first `GET /exchanges` page returns (default none). */
  rows?: () => Row[];
  health?: Record<string, unknown>;
}

export async function mockApp(page: Page, opts: MockOptions = {}): Promise<void> {
  const rows = opts.rows ?? (() => []);
  await page.route('**/api/v1/counts', (r) => r.fulfill({ status: 200, json: COUNTS }));
  await page.route('**/api/v1/health', (r) =>
    r.fulfill({ status: 200, json: opts.health ?? HEALTH }),
  );
  await page.route('**/api/v1/scope', (r) =>
    r.fulfill({ status: 200, json: { include: [], exclude: [] } }),
  );
  await page.route(/\/api\/v1\/repeater\/tabs$/, (r) => r.fulfill({ status: 200, json: [] }));
  await page.route(/\/api\/v1\/intercept$/, (r) =>
    r.fulfill({ status: 200, json: { enabled: false, rules: RULES, held: [] } }),
  );
  await page.route(/\/api\/v1\/exchanges(\?.*)?$/, (r) =>
    r.fulfill({ status: 200, json: { items: rows(), next_cursor: null } }),
  );
}

/**
 * Serve `/api/v1/events` in-page. Returns the live sockets, newest last, so a
 * spec can push frames or close one to simulate a drop.
 */
export async function mockEvents(page: Page): Promise<WebSocketRoute[]> {
  const sockets: WebSocketRoute[] = [];
  await page.routeWebSocket(/\/api\/v1\/events$/, (ws) => {
    sockets.push(ws);
  });
  return sockets;
}
