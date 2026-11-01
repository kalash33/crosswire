import { expect, type Page, type Request } from '@playwright/test';

// Repeater bench mock shared by repeater-bench.spec.ts and the XSS corpus:
// sends answer from a scripted list, exchanges and response bodies are served
// by exchange id, and drafts are kept in memory so a reload reads back what
// the page saved.

const COUNTS = {
  bytes_inline: 0, bytes_packs: 0, committed: 0, error: 0, exchanges: 0,
  history: 0, intruder: 0, markers: 0, repeater: 0,
};

export const b64 = (s: string) => Buffer.from(s, 'latin1').toString('base64');

export interface Resp {
  status: number;
  body: Buffer;
  /** The whole response head (status line, headers, blank line); a plain one by default. */
  head?: string;
}

/** A failed send: the HTTP status of `POST /repeater/send` and its error message. */
export interface SendError {
  error: number;
  message?: string;
}

export interface Draft {
  json: Record<string, unknown>;
  updated_at: number;
}

export interface Bench {
  sends: Request[];
  puts: { id: string; json: Record<string, unknown> }[];
  drafts: Map<string, Draft>;
  exchangeGets: string[];
}

export const exId = (n: number) => `01HZZZZZZZZZZZZZZZZZZZBEN${String.fromCharCode(65 + n)}`;

export function exchangeJson(id: string, resp: Resp) {
  return {
    id,
    request: {
      kind: 'http1-raw', fidelity: 'raw',
      head: { type: 'http1-raw', bytes: b64('GET / HTTP/1.1\r\nHost: api.example.test\r\n\r\n'), parser_version: 1 },
      diagnostics: [], body: { type: 'none' },
    },
    response: {
      kind: 'http1-raw', fidelity: 'raw',
      head: { type: 'http1-raw', bytes: b64(resp.head ?? `HTTP/1.1 ${resp.status} X\r\nContent-Type: text/plain\r\n\r\n`), parser_version: 1 },
      diagnostics: [], body: { type: 'inline', truncated_at: null },
    },
    scope_state: 'in', state: 'committed', tags: [], notes: '',
  };
}

/**
 * Mock the API. `responses[n]` answers the n-th send (a SendError fails the
 * send itself). Drafts are kept so a reload sees them.
 */
export async function mockBench(
  page: Page,
  responses: (Resp | SendError)[],
  seed: { tab_id: string; json: Record<string, unknown> }[] = [],
  extra: Record<string, Resp> = {},
): Promise<Bench> {
  const bench: Bench = { sends: [], puts: [], drafts: new Map(), exchangeGets: [] };
  for (const d of seed) bench.drafts.set(d.tab_id, { json: d.json, updated_at: 1 });
  const byId = new Map<string, Resp>(Object.entries(extra));
  await page.route('**/api/v1/counts', (r) => r.fulfill({ status: 200, json: COUNTS }));
  await page.route(/\/api\/v1\/repeater\/tabs$/, (r) =>
    r.fulfill({
      status: 200,
      json: [...bench.drafts].map(([tab_id, d]) => ({ tab_id, json: d.json, updated_at: d.updated_at })),
    }),
  );
  await page.route(/\/api\/v1\/repeater\/tabs\/[^/]+$/, (r) => {
    const id = decodeURIComponent(new URL(r.request().url()).pathname.split('/').pop()!);
    if (r.request().method() === 'PUT') {
      const json = r.request().postDataJSON() as Record<string, unknown>;
      bench.puts.push({ id, json });
      bench.drafts.set(id, { json, updated_at: Date.now() });
    } else if (r.request().method() === 'DELETE') bench.drafts.delete(id);
    return r.fulfill({ status: 200, body: '' });
  });
  await page.route(/\/api\/v1\/repeater\/send$/, (r) => {
    const n = bench.sends.length;
    bench.sends.push(r.request());
    const resp = responses[n];
    if (resp === undefined || 'error' in resp) {
      return r.fulfill({
        status: resp?.error ?? 500,
        json: { error: { code: 'upstream', message: resp?.message ?? 'upstream connect failed' } },
      });
    }
    byId.set(exId(n), resp);
    return r.fulfill({
      status: 200,
      json: {
        exchange_id: exId(n), scope_state: 'in', changes: [], skipped: [], cert_override: false,
        ttfb_ms: 10 + n, total_ms: 20 + n, redirects: [],
      },
    });
  });
  await page.route(/\/api\/v1\/exchanges\/[^/?]+$/, (r) => {
    const id = decodeURIComponent(new URL(r.request().url()).pathname.split('/').pop()!);
    bench.exchangeGets.push(id);
    const resp = byId.get(id);
    return resp
      ? r.fulfill({ status: 200, json: exchangeJson(id, resp) })
      : r.fulfill({ status: 404, json: { error: { code: 'not_found', message: 'no such exchange' } } });
  });
  await page.route(/\/api\/v1\/exchanges\/[^/]+\/body\/resp$/, (r) => {
    const id = decodeURIComponent(new URL(r.request().url()).pathname.split('/').at(-3)!);
    const resp = byId.get(id);
    return resp
      ? r.fulfill({
          status: 200,
          contentType: 'application/octet-stream',
          headers: { 'x-crosswire-length': String(resp.body.length) },
          body: resp.body,
        })
      : r.fulfill({ status: 404, json: { error: { code: 'not_found', message: 'no body' } } });
  });
  return bench;
}

export async function openRepeater(page: Page): Promise<void> {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/');
  await expect(page.getByTestId('app-shell')).toBeVisible();
  await page.getByTestId('nav-repeater').click();
  await expect(page.getByTestId('repeater-view')).toBeVisible();
}

