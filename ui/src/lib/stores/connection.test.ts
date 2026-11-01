import { afterEach, describe, expect, it, vi } from 'vitest';
import { createConnection, parseProject } from './connection.svelte';

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

describe('project', () => {
  it('takes a reported project and keeps it null when absent', async () => {
    const health = { status: 'ok', schema_version: 3, format_version: 7 };
    vi.stubGlobal(
      'fetch',
      vi
        .fn()
        .mockResolvedValueOnce(new Response(JSON.stringify(health), { status: 200 }))
        .mockResolvedValueOnce(
          new Response(
            JSON.stringify({ ...health, project: { name: 'acme', path: '/work/acme' } }),
            { status: 200 },
          ),
        ),
    );
    const absent = createConnection();
    await absent.load();
    expect(absent.project).toBeNull();
    const named = createConnection();
    await named.load();
    expect(named.project).toEqual({ name: 'acme', path: '/work/acme' });
  });

  it('drops a malformed or oversized project', () => {
    expect(parseProject(undefined)).toBeNull();
    expect(parseProject('acme')).toBeNull();
    expect(parseProject({ name: 'acme' })).toBeNull();
    expect(parseProject({ name: 1, path: '/a' })).toBeNull();
    expect(parseProject({ name: ' ', path: '/a' })).toBeNull();
    expect(parseProject({ name: 'a'.repeat(256), path: '/a' })).toBeNull();
    expect(parseProject({ name: 'a', path: '/'.repeat(4097) })).toBeNull();
    const xss = '<img src=x onerror=alert(1)>';
    expect(parseProject({ name: xss, path: `/w/${xss}`, extra: 1 })).toEqual({
      name: xss,
      path: `/w/${xss}`,
    });
  });
});
