import { describe, it, expect, vi } from 'vitest';
import { clientFilter, createHistory, type ExchangeRowDto, type PageDto } from './history.svelte';
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
const sync = (fn: () => void) => fn();

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
    const h = createHistory({ get, subscribe, schedule: sync });

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
    const h = createHistory({ get, subscribe, patch, maxRows: 2, schedule: sync });
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


describe('history store: window support', () => {
  function sub() {
    let emit: (e: Event) => void = () => {};
    return { subscribe: (oe: (e: Event) => void) => ((emit = oe), () => {}), emit: (e: Event) => emit(e) };
  }
  const recv = (id: string) => ({ type: 'received', id, seq: 1 }) as Event;

  it('two concurrent loadMore calls issue one GET', async () => {
    let resolve: (p: PageDto<ExchangeRowDto>) => void = () => {};
    const get = vi
      .fn()
      .mockResolvedValueOnce(page([row('A')], 'c1'))
      .mockImplementationOnce(() => new Promise((r) => (resolve = r)));
    const h = createHistory({ get, subscribe: noSub });
    await h.applyFilter({});
    const a = h.loadMore();
    const b = h.loadMore();
    resolve(page([row('B')], null));
    await Promise.all([a, b]);
    expect(get).toHaveBeenCalledTimes(2);
    expect(h.rows.map((r) => r.id)).toEqual(['A', 'B']);
  });

  it('appends a page that repeats a loaded id once', async () => {
    const get = vi
      .fn()
      .mockResolvedValueOnce(page([row('A'), row('B')], 'c1'))
      .mockResolvedValueOnce(page([row('B'), row('C'), row('C')], null));
    const h = createHistory({ get, subscribe: noSub });
    await h.applyFilter({});
    await h.loadMore();
    expect(h.rows.map((r) => r.id)).toEqual(['A', 'B', 'C']);
  });

  it('stops paging at maxRows and reports capped', async () => {
    const get = vi
      .fn()
      .mockResolvedValueOnce(page([row('A'), row('B')], 'c1'))
      .mockResolvedValueOnce(page([row('C'), row('D')], 'c2'))
      .mockResolvedValue(page([row('E')], null));
    const h = createHistory({ get, subscribe: noSub, maxRows: 4, pageSize: 2 });
    await h.applyFilter({});
    expect(h.capped).toBe(false);
    await h.loadMore();
    expect(h.rows).toHaveLength(4);
    expect(h.capped).toBe(true);
    await h.loadMore();
    expect(get).toHaveBeenCalledTimes(2);
    expect(h.rows).toHaveLength(4);
  });

  it('a prepend at the cap trims the tail and stays capped', async () => {
    const s = sub();
    const get = vi.fn().mockResolvedValue(page([row('A'), row('B'), row('C'), row('D')], 'c1'));
    const h = createHistory({ get, subscribe: s.subscribe, maxRows: 4, schedule: sync });
    await h.applyFilter({});
    expect(h.capped).toBe(true);
    s.emit(recv('N'));
    expect(h.rows.map((r) => r.id)).toEqual(['N', 'A', 'B', 'C']);
    expect(h.capped).toBe(true);
  });

  it('generation bumps on applyFilter and reload, not on loadMore or events', async () => {
    const s = sub();
    const get = vi
      .fn()
      .mockResolvedValueOnce(page([row('A')], 'c1'))
      .mockResolvedValue(page([row('B')], null));
    const h = createHistory({ get, subscribe: s.subscribe, schedule: sync });
    const g0 = h.generation;
    await h.applyFilter({});
    const g1 = h.generation;
    expect(g1).toBeGreaterThan(g0);
    await h.loadMore();
    s.emit(recv('N'));
    s.emit({ type: 'committed', ids: ['N'], seq: 2 } as Event);
    expect(h.generation).toBe(g1);
    await h.reload();
    expect(h.generation).toBeGreaterThan(g1);
  });

  it('coalesces a burst of received events into one rows change per flush', async () => {
    const s = sub();
    const queue: (() => void)[] = [];
    const get = vi.fn().mockResolvedValue(page([row('A')]));
    const h = createHistory({ get, subscribe: s.subscribe, schedule: (fn) => queue.push(fn) });
    await h.applyFilter({});
    const before = h.rows;
    for (const id of ['1', '2', '3', '4', '5']) s.emit(recv(id));
    expect(h.rows).toBe(before);
    expect(queue).toHaveLength(1);
    queue.shift()!();
    expect(h.rows).not.toBe(before);
    expect(h.rows.map((r) => r.id)).toEqual(['5', '4', '3', '2', '1', 'A']);
    expect(queue).toHaveLength(0);
  });

  it('indexOf and byId follow prepends and appends', async () => {
    const s = sub();
    const get = vi
      .fn()
      .mockResolvedValueOnce(page([row('A'), row('B')], 'c1'))
      .mockResolvedValueOnce(page([row('C')], null));
    const h = createHistory({ get, subscribe: s.subscribe, schedule: sync });
    await h.applyFilter({});
    expect(h.indexOf('B')).toBe(1);
    s.emit(recv('N'));
    expect(h.indexOf('N')).toBe(0);
    expect(h.indexOf('B')).toBe(2);
    await h.loadMore();
    expect(h.indexOf('C')).toBe(3);
    expect(h.byId('C')?.id).toBe('C');
    expect(h.indexOf('ZZ')).toBe(-1);
    expect(h.byId('ZZ')).toBeNull();
  });
});

describe('history store: races and teardown', () => {
  function sub() {
    let emit: (e: Event) => void = () => {};
    return { subscribe: (oe: (e: Event) => void) => ((emit = oe), () => {}), emit: (e: Event) => emit(e) };
  }

  it('a stale loadMore resolved after applyFilter is dropped and loading stays true', async () => {
    let resolveStale: (p: PageDto<ExchangeRowDto>) => void = () => {};
    let resolveNew: (p: PageDto<ExchangeRowDto>) => void = () => {};
    const get = vi
      .fn()
      .mockResolvedValueOnce(page([row('A')], 'c1'))
      .mockImplementationOnce(() => new Promise((r) => (resolveStale = r)))
      .mockImplementationOnce(() => new Promise((r) => (resolveNew = r)));
    const h = createHistory({ get, subscribe: noSub });
    await h.applyFilter({});
    const more = h.loadMore();
    const filt = h.applyFilter({ host: 'n.test' });
    resolveStale(page([row('OLD')], null));
    await more;
    expect(h.rows.map((r) => r.id)).toEqual(['A']);
    expect(h.loading).toBe(true);
    resolveNew(page([row('NEW')], null));
    await filt;
    expect(h.rows.map((r) => r.id)).toEqual(['NEW']);
    expect(h.loading).toBe(false);
  });

  it('dispose cancels a pending flush', async () => {
    const s = sub();
    const queue: (() => void)[] = [];
    const h = createHistory({
      get: vi.fn().mockResolvedValue(page([row('A')])),
      subscribe: s.subscribe,
      schedule: (fn) => queue.push(fn),
    });
    await h.applyFilter({});
    const before = h.rows;
    s.emit({ type: 'received', id: 'N', seq: 1 } as Event);
    h.dispose();
    queue.shift()!();
    expect(h.rows).toBe(before);
  });

  it('a committed event for an unloaded id is a no-op', async () => {
    const s = sub();
    const h = createHistory({
      get: vi.fn().mockResolvedValue(page([row('A')])),
      subscribe: s.subscribe,
      schedule: sync,
    });
    await h.applyFilter({});
    const before = h.rows;
    s.emit({ type: 'committed', ids: ['ZZ'], seq: 2 } as Event);
    expect(h.rows).toBe(before);
  });

  it('drops a page that was in flight when a flush trimmed the tail', async () => {
    const s = sub();
    let resolve: (p: PageDto<ExchangeRowDto>) => void = () => {};
    const get = vi
      .fn()
      .mockResolvedValueOnce(page([row('A'), row('B'), row('C')], 'c1'))
      .mockImplementationOnce(() => new Promise((r) => (resolve = r)));
    const h = createHistory({ get, subscribe: s.subscribe, maxRows: 4, schedule: sync });
    await h.applyFilter({});
    const more = h.loadMore();
    s.emit({ type: 'received', id: 'N1', seq: 1 } as Event);
    s.emit({ type: 'received', id: 'N2', seq: 2 } as Event);
    resolve(page([row('D')], 'c2'));
    await more;
    expect(h.rows.map((r) => r.id)).toEqual(['N2', 'N1', 'A', 'B']);
    expect(h.capped).toBe(true);
    expect(h.loading).toBe(false);
  });
});

describe('clientFilter', () => {
  const rows = [
    { host: 'Api.Example.test', path: '/Login', tags: ['auth'] },
    { host: 'b.test', path: '/x', tags: ['IDOR', 'todo'] },
    { host: null, path: null, tags: [] },
  ];

  it('keeps every row for a blank query', () => {
    expect(clientFilter(rows, '')).toHaveLength(3);
    expect(clientFilter(rows, '   ')).toHaveLength(3);
  });

  it('matches host, path and tags case-insensitively as substrings', () => {
    expect(clientFilter(rows, 'EXAMPLE')).toEqual([rows[0]]);
    expect(clientFilter(rows, 'login')).toEqual([rows[0]]);
    expect(clientFilter(rows, 'idor')).toEqual([rows[1]]);
    expect(clientFilter(rows, 'od')).toEqual([rows[1]]);
  });

  it('matches nothing for an absent needle and never throws on null fields', () => {
    expect(clientFilter(rows, 'zzz')).toEqual([]);
  });

  it('treats hostile strings as plain text, not patterns', () => {
    const hostile = [
      { host: '<img src=x onerror=alert(1)>', path: '/a', tags: ['"><script>alert(1)</script>'] },
      { host: 'plain.test', path: '/.*', tags: [] },
    ];
    expect(clientFilter(hostile, '<img src=x')).toEqual([hostile[0]]);
    expect(clientFilter(hostile, '<script>')).toEqual([hostile[0]]);
    expect(clientFilter(hostile, '.*')).toEqual([hostile[1]]);
    expect(clientFilter(hostile, '(')).toEqual([hostile[0]]);
  });
});
