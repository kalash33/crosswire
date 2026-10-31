import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { session } from './session.svelte';

function mockFetch(status: number, body: unknown) {
  const fn = vi.fn(async () => new Response(JSON.stringify(body), { status }));
  vi.stubGlobal('fetch', fn);
  return fn;
}

beforeEach(() => {
  session.status = 'unknown';
  history.replaceState(null, '', '/');
});
afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe('bootstrapFromFragment', () => {
  it('exchanges the token, clears the fragment, and authenticates', async () => {
    history.replaceState(null, '', '/?x=1#bootstrap=tok123');
    const spy = vi.spyOn(history, 'replaceState');
    const fn = mockFetch(200, { expires_in: 1800 });
    await session.bootstrapFromFragment();
    const [url, init] = fn.mock.calls[0] as unknown as [string, RequestInit];
    expect(url).toBe('/api/v1/session');
    expect(init.method).toBe('POST');
    expect(JSON.parse(init.body as string)).toEqual({ bootstrap: 'tok123' });
    expect(spy).toHaveBeenCalledWith(null, '', '/?x=1');
    expect(location.hash).toBe('');
    expect(session.status).toBe('authenticated');
  });

  it('marks a reused/invalid token unauthenticated and still clears the fragment', async () => {
    history.replaceState(null, '', '/#bootstrap=used');
    mockFetch(401, { error: { code: 'unauthorized', message: 'bad token' } });
    await session.bootstrapFromFragment();
    expect(session.status).toBe('unauthenticated');
    expect(location.hash).toBe('');
  });

  it('treats a present-but-empty fragment as unauthenticated and clears it', async () => {
    history.replaceState(null, '', '/#bootstrap=');
    const fn = mockFetch(200, {});
    await session.bootstrapFromFragment();
    expect(fn).not.toHaveBeenCalled();
    expect(session.status).toBe('unauthenticated');
    expect(location.hash).toBe('');
  });

  it('does not throw on a malformed %-escape and never stays unknown', async () => {
    history.replaceState(null, '', '/#bootstrap=%zz');
    mockFetch(401, { error: { code: 'unauthorized', message: 'bad' } });
    await session.bootstrapFromFragment();
    expect(session.status).toBe('unauthenticated');
    expect(location.hash).toBe('');
  });

  it('does nothing without a fragment token', async () => {
    const fn = mockFetch(200, {});
    await session.bootstrapFromFragment();
    expect(fn).not.toHaveBeenCalled();
    expect(session.status).toBe('unknown');
  });
});

describe('submitToken', () => {
  it('accepts a raw token', async () => {
    const fn = mockFetch(200, { expires_in: 1 });
    await session.submitToken('  rawtok  ');
    const [, init] = fn.mock.calls[0] as unknown as [string, RequestInit];
    expect(JSON.parse(init.body as string)).toEqual({ bootstrap: 'rawtok' });
    expect(session.status).toBe('authenticated');
  });

  it('extracts the token from a pasted bootstrap URL', async () => {
    const fn = mockFetch(200, { expires_in: 1 });
    await session.submitToken('http://localhost:8080/#bootstrap=abc_DEF-9');
    const [, init] = fn.mock.calls[0] as unknown as [string, RequestInit];
    expect(JSON.parse(init.body as string)).toEqual({ bootstrap: 'abc_DEF-9' });
  });

  it('reports failure as unauthenticated', async () => {
    mockFetch(401, { error: { code: 'unauthorized', message: 'nope' } });
    await session.submitToken('bad');
    expect(session.status).toBe('unauthenticated');
    expect(session.error).toBe('nope');
  });
});

describe('submitToken malformed', () => {
  it('survives a malformed %-escape in a pasted URL', async () => {
    mockFetch(401, { error: { code: 'unauthorized', message: 'bad' } });
    await session.submitToken('http://x/#bootstrap=%zz');
    expect(session.status).toBe('unauthenticated');
  });
});

describe('probe', () => {
  it('maps 401 to unauthenticated', async () => {
    mockFetch(401, { error: { code: 'unauthorized', message: 'no session' } });
    await session.probe();
    expect(session.status).toBe('unauthenticated');
  });
  it('maps 200 to authenticated', async () => {
    mockFetch(200, {});
    await session.probe();
    expect(session.status).toBe('authenticated');
  });
  it('treats other errors as unauthenticated without clobbering an authenticated state', async () => {
    mockFetch(500, {});
    await session.probe();
    expect(session.status).toBe('unauthenticated');
    session.status = 'authenticated';
    await session.probe();
    expect(session.status).toBe('authenticated');
  });
});

describe('expire (mid-session 401)', () => {
  it('flips an authenticated session to unauthenticated with a message', () => {
    session.status = 'authenticated';
    session.error = null;
    session.expire();
    expect(session.status).toBe('unauthenticated');
    expect(session.error).toMatch(/expired/i);
  });

  it('a 401 from any authenticated API call expires the session', async () => {
    session.status = 'authenticated';
    mockFetch(401, { error: { code: 'unauthorized', message: 'no session' } });
    const { apiGet } = await import('../api/client');
    await expect(apiGet('/api/v1/scope')).rejects.toThrow();
    expect(session.status).toBe('unauthenticated');
  });

  it('a 401 from the bootstrap exchange is not an expiry', async () => {
    session.status = 'unauthenticated';
    session.error = null;
    mockFetch(401, { error: { code: 'unauthorized', message: 'bad token' } });
    await session.submitToken('nope');
    expect(session.error).toBe('bad token');
  });
});
