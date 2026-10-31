import { describe, it, expect, vi } from 'vitest';
import { createHistory, type ExchangeRowDto, type PageDto } from './history.svelte';
import type { Event } from './events.svelte';

function page(items: ExchangeRowDto[], next_cursor: string | null = null): PageDto<ExchangeRowDto> {
  return { items, next_cursor };
}

function row(id: string, over: Partial<ExchangeRowDto> = {}): ExchangeRowDto {
  return {
    id,
    created_at: 1_700_000_000_000,
    method: 'GET',
    host: 'a.test',
    path: '/',
    status: 200,
    resp_len: 1,
    state: 'committed',
    tags: [],
    ...over,
  };
}

const noSub = () => () => {};

describe('history store', () => {
  it('applyFilter builds the right query params and reloads from the first page', async () => {
    const get = vi.fn().mockResolvedValue(page([row('A')]));
    const h = createHistory({ get, subscribe: noSub });

    await h.applyFilter({
      host: 'a.test',
      host_contains: 'a',
      method: 'GET',
      status_min: 200,
      status_max: 299,
      tag: 'x',
      state: 'committed',
      scope_state: 'in',
      namespace: 'history',
    });

    expect(get).toHaveBeenCalledTimes(1);
    const [path, params] = get.mock.calls[0] as [string, Record<string, unknown>];
    expect(path).toBe('/api/v1/exchanges');
    expect(params).toMatchObject({
      host: 'a.test',
      host_contains: 'a',
      method: 'GET',
      status_min: 200,
      status_max: 299,
      tag: 'x',
      state: 'committed',
      scope_state: 'in',
      namespace: 'history',
    });
    // First page carries no keyset cursor.
    expect(params.after).toBeUndefined();
    expect(typeof params.limit).toBe('number');
    expect(h.rows.map((r) => r.id)).toEqual(['A']);
  });

  it('loadMore advances the keyset cursor (uses next_cursor as after) and appends', async () => {
    const get = vi
      .fn()
      .mockResolvedValueOnce(page([row('A')], 'cur-1'))
      .mockResolvedValueOnce(page([row('B')], null));
    const h = createHistory({ get, subscribe: noSub });

    await h.applyFilter({});
    expect(h.hasMore).toBe(true);

    await h.loadMore();
    expect(get).toHaveBeenCalledTimes(2);
    expect((get.mock.calls[1][1] as Record<string, unknown>).after).toBe('cur-1');
    expect(h.rows.map((r) => r.id)).toEqual(['A', 'B']);
    expect(h.hasMore).toBe(false);

    // Exhausted: no further request.
    await h.loadMore();
    expect(get).toHaveBeenCalledTimes(2);
  });

  it('a committed event flips a row uncommitted flag', async () => {
    let onEvent: ((e: Event) => void) | undefined;
    const subscribe = (oe: (e: Event) => void) => {
      onEvent = oe;
      return () => {};
    };
    const get = vi.fn().mockResolvedValue(page([row('A', { state: 'committed' })]));
    const h = createHistory({ get, subscribe });

    await h.applyFilter({});
    expect(h.rows.find((r) => r.id === 'A')?.uncommitted).toBe(false);

    onEvent?.({ type: 'received', id: 'A', seq: 1 });
    expect(h.rows.find((r) => r.id === 'A')?.uncommitted).toBe(true);

    onEvent?.({ type: 'committed', ids: ['A'], seq: 2 });
    expect(h.rows.find((r) => r.id === 'A')?.uncommitted).toBe(false);
  });

  it('onLagged refetches the current view from the first page, keeping the filter', async () => {
    let onLagged: (() => void) | undefined;
    const subscribe = (_oe: (e: Event) => void, ol: () => void) => {
      onLagged = ol;
      return () => {};
    };
    const get = vi.fn().mockResolvedValue(page([row('A')], 'cur-1'));
    const h = createHistory({ get, subscribe });

    await h.applyFilter({ host: 'a.test' });
    expect(get).toHaveBeenCalledTimes(1);

    onLagged?.();
    // Let the async refetch settle.
    await Promise.resolve();
    await Promise.resolve();

    expect(get).toHaveBeenCalledTimes(2);
    const refetch = get.mock.calls[1][1] as Record<string, unknown>;
    expect(refetch.host).toBe('a.test');
    expect(refetch.after).toBeUndefined();
  });

  it('patchRow_updates_only_matching_row', async () => {
    const get = vi.fn().mockResolvedValue(page([row('A'), row('B')]));
    const h = createHistory({ get, subscribe: noSub });
    await h.reload();
    h.patchRow('B', { tags: ['x'], notes: 'n' });
    expect(h.rows.find((r) => r.id === 'A')?.tags).toEqual([]);
    expect(h.rows.find((r) => r.id === 'B')?.tags).toEqual(['x']);
  });

  it('patchRow_unknown_id_is_noop', async () => {
    const get = vi.fn().mockResolvedValue(page([row('A')]));
    const h = createHistory({ get, subscribe: noSub });
    await h.reload();
    const before = h.rows;
    h.patchRow('ZZ', { tags: ['x'] });
    expect(h.rows).toBe(before);
  });

  it('bulkTag issues one PATCH per id, in order, appending to and deduping each row', async () => {
    const get = vi.fn().mockResolvedValue(
      page([row('A', { tags: ['x'] }), row('B', { tags: ['keep', 'x'] }), row('C', { tags: [] })]),
    );
    const calls: [string, unknown][] = [];
    const patch = vi.fn(async (id: string, body: unknown) => {
      calls.push([id, body]);
    });
    const h = createHistory({ get, subscribe: noSub, patch });
    await h.applyFilter({});

    const res = await h.bulkTag(['C', 'A', 'B'], ' x ');

    expect(calls).toEqual([
      ['C', { tags: ['x'] }],
      ['A', { tags: ['x'] }],
      ['B', { tags: ['keep', 'x'] }],
    ]);
    expect(res).toEqual({ ok: ['C', 'A', 'B'], failed: [] });
    expect(h.rows.find((r) => r.id === 'C')?.tags).toEqual(['x']);
  });

  it('bulkTag waits for each PATCH before the next and reports failures without stopping', async () => {
    const get = vi.fn().mockResolvedValue(page([row('A'), row('B')]));
    let inFlight = 0;
    let maxInFlight = 0;
    const patch = vi.fn(async (id: string) => {
      inFlight++;
      maxInFlight = Math.max(maxInFlight, inFlight);
      await new Promise((r) => setTimeout(r, 1));
      inFlight--;
      if (id === 'A') throw new Error('nope');
    });
    const h = createHistory({ get, subscribe: noSub, patch });
    await h.applyFilter({});
    const res = await h.bulkTag(['A', 'B'], 'y');
    expect(maxInFlight).toBe(1);
    expect(res).toEqual({ ok: ['B'], failed: [{ id: 'A', error: 'nope' }] });
    expect(h.rows.find((r) => r.id === 'A')?.tags).toEqual([]);
    expect(h.rows.find((r) => r.id === 'B')?.tags).toEqual(['y']);
  });

  it('bulkTag rejects an invalid tag without any PATCH', async () => {
    const patch = vi.fn();
    const h = createHistory({ get: vi.fn().mockResolvedValue(page([row('A')])), subscribe: noSub, patch });
    await h.applyFilter({});
    await expect(h.bulkTag(['A'], '   ')).rejects.toThrow('tag is empty');
    expect(patch).not.toHaveBeenCalled();
  });

  it('bulkAddHosts returns the unique hosts of the given rows in first-seen order', async () => {
    const get = vi.fn().mockResolvedValue(
      page([row('A', { host: 'b.test' }), row('B', { host: 'a.test' }), row('C', { host: 'b.test' }), row('D', { host: null })]),
    );
    const h = createHistory({ get, subscribe: noSub });
    await h.applyFilter({});
    expect(h.bulkAddHosts(['A', 'B', 'C', 'D', 'ZZ'])).toEqual(['b.test', 'a.test']);
  });

  it('bulkTag reports an id with no loaded row as failed and never PATCHes it from []', async () => {
    const patch = vi.fn().mockResolvedValue(undefined);
    const getExchange = vi.fn();
    const h = createHistory({ get: vi.fn().mockResolvedValue(page([row('A')])), subscribe: noSub, patch, getExchange });
    await h.applyFilter({});
    const res = await h.bulkTag(['GONE', 'A'], 't');
    expect(res.failed).toEqual([{ id: 'GONE', error: 'row no longer loaded; reload and retry' }]);
    expect(res.ok).toEqual(['A']);
    expect(patch.mock.calls.map((c) => c[0])).toEqual(['A']);
    expect(getExchange).not.toHaveBeenCalled();
  });

  it('bulkTag snapshots tags before the first await, so a row trimmed mid-run keeps its tags', async () => {
    let emit: (e: Event) => void = () => {};
    const subscribe = (onEvent: (e: Event) => void) => {
      emit = onEvent;
      return () => {};
    };
    const get = vi.fn().mockResolvedValue(page([row('A'), row('B', { tags: ['keep'] })]));
    const patch = vi.fn(async (id: string) => {
      // A live flood during the first PATCH pushes B out of the 2-row cap.
      if (id === 'A') {
        emit({ type: 'received', id: 'N1' } as Event);
        emit({ type: 'received', id: 'N2' } as Event);
      }
    });
    const h = createHistory({ get, subscribe, patch, maxRows: 2 });
    await h.applyFilter({});
    const res = await h.bulkTag(['A', 'B'], 't');
    expect(h.rows.some((r) => r.id === 'B')).toBe(false);
    expect(patch.mock.calls).toEqual([
      ['A', { tags: ['t'] }],
      ['B', { tags: ['keep', 't'] }],
    ]);
    expect(res.ok).toEqual(['A', 'B']);
  });

  it('bulkTag reads a placeholder/uncommitted row from the server before PATCHing it', async () => {
    const order: string[] = [];
    const get = vi.fn().mockResolvedValue(
      page([row('A', { tags: ['a'] }), row('P', { method: null, state: 'received' }), row('C', { tags: ['c'] })]),
    );
    const getExchange = vi.fn(async (id: string) => {
      order.push(`get:${id}`);
      return { tags: ['server-only'] };
    });
    const patch = vi.fn(async (id: string, body: unknown) => {
      order.push(`patch:${id}:${JSON.stringify(body)}`);
    });
    const h = createHistory({ get, subscribe: noSub, patch, getExchange });
    await h.applyFilter({});
    const res = await h.bulkTag(['A', 'P', 'C'], 'n');
    expect(order).toEqual([
      'patch:A:{"tags":["a","n"]}',
      'get:P',
      'patch:P:{"tags":["server-only","n"]}',
      'patch:C:{"tags":["c","n"]}',
    ]);
    expect(res.ok).toEqual(['A', 'P', 'C']);
  });

  it('bulkTag fails (without PATCH) a placeholder whose server read fails', async () => {
    const patch = vi.fn().mockResolvedValue(undefined);
    const getExchange = vi.fn().mockRejectedValue(new Error('offline'));
    const h = createHistory({
      get: vi.fn().mockResolvedValue(page([row('P', { method: null, state: 'received' })])),
      subscribe: noSub,
      patch,
      getExchange,
    });
    await h.applyFilter({});
    const res = await h.bulkTag(['P'], 'n');
    expect(res).toEqual({ ok: [], failed: [{ id: 'P', error: 'could not read current tags: offline' }] });
    expect(patch).not.toHaveBeenCalled();
  });

  it('bulkTag waits for an in-flight editor tag save, then re-reads that exchange', async () => {
    const order: string[] = [];
    let release: () => void = () => {};
    const saving = new Promise<void>((r) => (release = r));
    const h = createHistory({
      get: vi.fn().mockResolvedValue(page([row('A', { tags: ['old'] })])),
      subscribe: noSub,
      patch: async (id, body) => void order.push(`patch:${id}:${JSON.stringify(body)}`),
      getExchange: async () => {
        order.push('get');
        return { tags: ['old', 'typed'] };
      },
      savePending: (id) => id === 'A',
      saveSettled: async () => {
        order.push('wait');
        await saving;
        order.push('settled');
      },
    });
    await h.applyFilter({});
    const run = h.bulkTag(['A'], 'bulk');
    await Promise.resolve();
    expect(order).toEqual(['wait']);
    release();
    await run;
    expect(order).toEqual(['wait', 'settled', 'get', 'patch:A:{"tags":["old","typed","bulk"]}']);
  });
});

