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
    const out = await patchExchange('X', { tags: ['a'] });
    expect(out.tags).toEqual(['a']);
    expect(calls[0].url).toBe('/api/v1/exchanges/X');
    expect(calls[0].init.method).toBe('PATCH');
    expect(JSON.parse(calls[0].init.body as string)).toEqual({ tags: ['a'] });
  });
});
