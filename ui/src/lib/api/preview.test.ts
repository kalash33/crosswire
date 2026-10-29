import { afterEach, describe, expect, it, vi } from 'vitest';
import { previewSrc, previewToken } from './preview';

afterEach(() => vi.unstubAllGlobals());

describe('preview', () => {
  it('mints via the preview-token route', async () => {
    const fn = vi.fn(
      async () =>
        new Response(JSON.stringify({ token: 't', path: '/preview/X?token=t' }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        }),
    );
    vi.stubGlobal('fetch', fn);
    const out = await previewToken('X');
    expect(out.path).toBe('/preview/X?token=t');
    expect((fn.mock.calls[0] as unknown[])[0]).toBe('/api/v1/exchanges/X/preview-token');
  });

  it('builds the src on the 127.0.0.1 origin', () => {
    expect(previewSrc('/preview/X?token=t', '8080')).toBe('http://127.0.0.1:8080/preview/X?token=t');
  });
});
