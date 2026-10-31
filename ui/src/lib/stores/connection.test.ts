import { afterEach, describe, expect, it, vi } from 'vitest';
import { createConnection } from './connection.svelte';

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('connection.load', () => {
  it('retries health after a failure instead of caching it', async () => {
    const fn = vi
      .fn()
      .mockResolvedValueOnce(new Response('{}', { status: 503 }))
      .mockResolvedValueOnce(
        new Response(JSON.stringify({ status: 'ok', schema_version: 3, format_version: 7 }), {
          status: 200,
        }),
      );
    vi.stubGlobal('fetch', fn);
    const c = createConnection();
    await c.load();
    expect(c.health).toBeNull();
    expect(c.error).not.toBeNull();
    await c.load();
    expect(fn).toHaveBeenCalledTimes(2);
    expect(c.health).toEqual({ schemaVersion: 3, formatVersion: 7, version: null });
    expect(c.error).toBeNull();
    await c.load();
    expect(fn).toHaveBeenCalledTimes(2); // success stays cached
  });

  it('takes a reported proxy address, mapping a wildcard bind to loopback', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async () =>
        new Response(
          JSON.stringify({ status: 'ok', schema_version: 3, format_version: 7, proxy_addr: '0.0.0.0:9090' }),
          { status: 200 },
        ),
      ),
    );
    const c = createConnection();
    await c.load();
    expect(c.proxyAddr).toBe('127.0.0.1:9090');
  });
});
