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
});
