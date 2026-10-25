import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { apiGet, apiSend } from './client';
import { ApiError, UnauthorizedError } from './errors';
import { MIRRORED_SCHEMAS } from './types';

function mockFetch(status: number, body: unknown, raw = false) {
  const fn = vi.fn(
    async () =>
      new Response(raw ? (body as string) : JSON.stringify(body), {
        status,
        headers: { 'content-type': 'application/json' },
      }),
  );
  vi.stubGlobal('fetch', fn);
  return fn;
}

async function rejection(p: Promise<unknown>): Promise<ApiError> {
  try {
    await p;
  } catch (e) {
    return e as ApiError;
  }
  throw new Error('expected the request to reject');
}

afterEach(() => vi.unstubAllGlobals());

describe('apiGet', () => {
  it('returns the parsed body on success and sends same-origin headers', async () => {
    const fn = mockFetch(200, { status: 'ok' });
    const out = await apiGet<{ status: string }>('/api/v1/health');
    expect(out).toEqual({ status: 'ok' });
    const [url, init] = fn.mock.calls[0] as unknown as [string, RequestInit];
    expect(url).toBe('/api/v1/health');
    expect(init.credentials).toBe('same-origin');
    expect(new Headers(init.headers).get('sec-fetch-site')).toBe('same-origin');
  });

  it('serializes params, repeating arrays and skipping undefined', async () => {
    const fn = mockFetch(200, {});
    await apiGet('/api/v1/exchanges', { limit: 5, tag: ['a', 'b'], q: undefined, x: null, on: false });
    const [url] = fn.mock.calls[0] as unknown as [string];
    expect(url).toBe('/api/v1/exchanges?limit=5&tag=a&tag=b&on=false');
  });

  it('rejects with ApiError carrying code/message/status', async () => {
    mockFetch(404, { error: { code: 'not_found', message: 'no such exchange' } });
    const err = await rejection(apiGet('/api/v1/exchanges/9'));
    expect(err).toBeInstanceOf(ApiError);
    expect(err).not.toBeInstanceOf(UnauthorizedError);
    expect(err.code).toBe('not_found');
    expect(err.message).toBe('no such exchange');
    expect(err.status).toBe(404);
  });

  it('rejects with UnauthorizedError on 401', async () => {
    mockFetch(401, { error: { code: 'unauthorized', message: 'no session' } });
    const err = await rejection(apiGet('/api/v1/counts'));
    expect(err).toBeInstanceOf(UnauthorizedError);
    expect(err).toBeInstanceOf(ApiError);
    expect(err.status).toBe(401);
    expect(err.code).toBe('unauthorized');
  });

  it('throws a fallback ApiError when the error body is not the envelope', async () => {
    mockFetch(502, '<html>bad gateway</html>', true);
    const err = await rejection(apiGet('/api/v1/counts'));
    expect(err).toBeInstanceOf(ApiError);
    expect(err.status).toBe(502);
    expect(err.code).toBe('unknown');
  });
});

describe('apiSend', () => {
  it('sends a JSON body with content-type and the method', async () => {
    const fn = mockFetch(200, { expires_in: 60 });
    const out = await apiSend<{ expires_in: number }>('POST', '/api/v1/session', { bootstrap: 't' });
    expect(out).toEqual({ expires_in: 60 });
    const [, init] = fn.mock.calls[0] as unknown as [string, RequestInit];
    expect(init.method).toBe('POST');
    expect(init.body).toBe('{"bootstrap":"t"}');
    const h = new Headers(init.headers);
    expect(h.get('content-type')).toBe('application/json');
    expect(h.get('sec-fetch-site')).toBe('same-origin');
    expect(init.credentials).toBe('same-origin');
  });

  it('resolves undefined on 204 and omits content-type without a body', async () => {
    const fn = vi.fn(async () => new Response(null, { status: 204 }));
    vi.stubGlobal('fetch', fn);
    const out = await apiSend('DELETE', '/api/v1/exchanges/1');
    expect(out).toBeUndefined();
    const [, init] = fn.mock.calls[0] as unknown as [string, RequestInit];
    expect(new Headers(init.headers).has('content-type')).toBe(false);
  });

  it('rejects with ApiError invalid_response on a 2xx non-JSON body', async () => {
    mockFetch(200, '<html>not json</html>', true);
    const err = await rejection(apiGet('/api/v1/health'));
    expect(err).toBeInstanceOf(ApiError);
    expect(err).not.toBeInstanceOf(SyntaxError);
    expect(err.code).toBe('invalid_response');
    expect(err.status).toBe(200);
  });

  it('rejects with ApiError on a non-2xx envelope', async () => {
    mockFetch(400, { error: { code: 'bad_request', message: 'nope' } });
    const err = await rejection(apiSend('PUT', '/api/v1/intercept/enabled', { enabled: true }));
    expect(err).toBeInstanceOf(ApiError);
    expect(err.code).toBe('bad_request');
    expect(err.status).toBe(400);
  });
});

describe('types match openapi.json', () => {
  // The real committed document. Vitest's cwd is ui/ (npm --prefix ui test),
  // so the repo-root doc is one level up. (import.meta.url is http: under jsdom.)
  const path = resolve(process.cwd(), '../docs/api/openapi.json');
  const doc = JSON.parse(readFileSync(path, 'utf8')) as {
    components: { schemas: Record<string, unknown> };
  };

  it('every schema name types.ts mirrors exists in components.schemas', () => {
    expect(MIRRORED_SCHEMAS.length).toBe(11);
    for (const name of MIRRORED_SCHEMAS) {
      expect(Object.keys(doc.components.schemas), name).toContain(name);
    }
  });

  it('the doc defines no named schema types.ts does not mirror', () => {
    expect(Object.keys(doc.components.schemas).sort()).toEqual([...MIRRORED_SCHEMAS].sort());
  });
});
