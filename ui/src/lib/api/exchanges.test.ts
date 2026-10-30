import { afterEach, describe, expect, it, vi } from 'vitest';
import { patchExchange } from './exchanges';

afterEach(() => vi.unstubAllGlobals());

describe('exchanges', () => {
  it('patchExchange_sends_PATCH_with_body', async () => {
    const calls: { url: string; init: RequestInit }[] = [];
    vi.stubGlobal('fetch', async (url: string, init: RequestInit) => {
      calls.push({ url, init });
      return new Response(JSON.stringify({ id: 'X', tags: ['a'] }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      });
    });
    await patchExchange('X', { tags: ['a'] });
    expect(calls[0].url).toBe('/api/v1/exchanges/X');
    expect(calls[0].init.method).toBe('PATCH');
    expect(JSON.parse(calls[0].init.body as string)).toEqual({ tags: ['a'] });
  });

  it('patchExchange_resolves_on_empty_200', async () => {
    vi.stubGlobal('fetch', async () => new Response('', { status: 200 }));
    await expect(patchExchange('X', { notes: 'n' })).resolves.toBeUndefined();
  });

  it('patchExchange_throws_on_non_2xx', async () => {
    vi.stubGlobal(
      'fetch',
      async () =>
        new Response(JSON.stringify({ error: { code: 'internal', message: 'boom' } }), {
          status: 500,
          headers: { 'content-type': 'application/json' },
        }),
    );
    await expect(patchExchange('X', { notes: 'n' })).rejects.toThrow();
  });
});
