import { describe, it, expect, vi } from 'vitest';
import { createScope, type ScopeApi } from './scope.svelte';
import { ApiError } from '../api/errors';
import type { Scope } from '../api/scope';

const LIVE: Scope = {
  include: [{ host: { type: 'exact', value: 'a.test' }, ports: [443], scheme: 'https' }],
  exclude: [{ host: { type: 'suffix', value: { suffix: 'x.test', also_apex: true } } }],
};
const NEXT: Scope = {
  include: [
    { host: { type: 'exact', value: 'a.test' } },
    { host: { type: 'cidr', value: '10.0.0.0/8' }, path: { type: 'prefix', value: '/api' } },
  ],
  exclude: [],
};

function mockApi(over: Partial<ScopeApi> = {}): ScopeApi {
  return {
    getScope: vi.fn().mockResolvedValue(LIVE),
    putScope: vi.fn().mockResolvedValue(undefined),
    importScope: vi.fn().mockResolvedValue(NEXT),
    setEnforceStrict: vi.fn().mockResolvedValue(undefined),
    ...over,
  };
}

describe('scope store', () => {
  it('load GETs the scope', async () => {
    const s = createScope({ api: mockApi() });
    await s.load();
    expect(s.scope).toEqual(LIVE);
    expect(s.loaded).toBe(true);
  });

  it('save PUTs the scope JSON and adopts it', async () => {
    const api = mockApi();
    const s = createScope({ api });
    await s.load();
    expect(await s.save(NEXT)).toBe(true);
    expect(api.putScope).toHaveBeenCalledWith(NEXT);
    expect(s.scope).toEqual(NEXT);
    expect(s.error).toBeNull();
  });

  it('a 422 scope_invalid surfaces the error and keeps the prior scope', async () => {
    const api = mockApi({
      putScope: vi.fn().mockRejectedValue(new ApiError('scope_invalid', 'invalid regex: (', 422)),
    });
    const s = createScope({ api });
    await s.load();
    expect(await s.save(NEXT)).toBe(false);
    expect(s.error).toBe('invalid regex: (');
    expect(s.scope).toEqual(LIVE);
  });

  it('importList posts {text} and previews without PUTting', async () => {
    const api = mockApi();
    const s = createScope({ api });
    await s.load();
    await s.importList('a.test\n10.0.0.0/8');
    expect(api.importScope).toHaveBeenCalledWith('a.test\n10.0.0.0/8');
    expect(s.preview).toEqual(NEXT);
    expect(api.putScope).not.toHaveBeenCalled();
    expect(s.scope).toEqual(LIVE);
  });

  it('a failed import leaves no preview and sets the error', async () => {
    const api = mockApi({
      importScope: vi.fn().mockRejectedValue(new ApiError('scope_invalid', 'import failed', 422)),
    });
    const s = createScope({ api });
    await s.importList('???');
    expect(s.preview).toBeNull();
    expect(s.error).toBe('import failed');
  });

  it('applyPreview PUTs the preview and clears it', async () => {
    const api = mockApi();
    const s = createScope({ api });
    await s.load();
    await s.importList('x');
    expect(await s.applyPreview()).toBe(true);
    expect(api.putScope).toHaveBeenCalledWith(NEXT);
    expect(s.scope).toEqual(NEXT);
    expect(s.preview).toBeNull();
  });

  it('applyPreview failing keeps the preview and the prior scope', async () => {
    const api = mockApi({
      putScope: vi.fn().mockRejectedValue(new ApiError('scope_invalid', 'bad', 422)),
    });
    const s = createScope({ api });
    await s.load();
    await s.importList('x');
    expect(await s.applyPreview()).toBe(false);
    expect(s.preview).toEqual(NEXT);
    expect(s.scope).toEqual(LIVE);
  });

  it('setEnforceStrict posts {enabled} and tracks it', async () => {
    const api = mockApi();
    const s = createScope({ api });
    expect(s.enforceStrict).toBe(false);
    await s.setEnforceStrict(true);
    expect(api.setEnforceStrict).toHaveBeenCalledWith(true);
    expect(s.enforceStrict).toBe(true);
    expect(s.strictKnown).toBe(true);
  });

  it('a failed setEnforceStrict keeps the prior value', async () => {
    const api = mockApi({ setEnforceStrict: vi.fn().mockRejectedValue(new Error('nope')) });
    const s = createScope({ api });
    await s.setEnforceStrict(true);
    expect(s.enforceStrict).toBe(false);
    expect(s.error).toBe('nope');
  });

  it('save and applyPreview never PUT before a successful load', async () => {
    const api = mockApi({ getScope: vi.fn().mockRejectedValue(new Error('down')) });
    const s = createScope({ api });
    await s.load();
    expect(s.loaded).toBe(false);
    expect(await s.save({ include: [], exclude: [] })).toBe(false);
    await s.importList('x');
    expect(await s.applyPreview()).toBe(false);
    expect(api.putScope).not.toHaveBeenCalled();
  });
});
