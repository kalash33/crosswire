import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createEventStore, type Event } from './events.svelte';

class MockSocket {
  static instances: MockSocket[] = [];
  onopen: (() => void) | null = null;
  onmessage: ((e: { data: string }) => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  closed = false;
  constructor(public url: string) {
    MockSocket.instances.push(this);
  }
  close() {
    this.closed = true;
  }
  open() {
    this.onopen?.();
  }
  send(frame: unknown) {
    this.onmessage?.({ data: JSON.stringify(frame) });
  }
  drop() {
    this.onclose?.();
  }
}

const factory = (url: string) => new MockSocket(url) as unknown as WebSocket;
const last = () => MockSocket.instances[MockSocket.instances.length - 1]!;

beforeEach(() => {
  MockSocket.instances = [];
  vi.useFakeTimers();
});
afterEach(() => {
  vi.useRealTimers();
});

describe('event store', () => {
  it('opens one same-origin socket to /api/v1/events', () => {
    const s = createEventStore({ socketFactory: factory });
    s.start();
    s.start();
    expect(MockSocket.instances).toHaveLength(1);
    const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
    expect(last().url).toBe(`${proto}//${location.host}/api/v1/events`);
    s.close();
  });

  it('delivers received/committed frames to onEvent', () => {
    const s = createEventStore({ socketFactory: factory });
    const onEvent = vi.fn();
    const onLagged = vi.fn();
    s.subscribe(onEvent, onLagged);
    s.start();
    last().open();
    last().send({ type: 'received', id: 'A', seq: 1 });
    last().send({ type: 'committed', ids: ['A'], seq: 2 });
    last().send({ type: 'intercept', held: 3 });
    expect(onEvent).toHaveBeenCalledTimes(3);
    expect((onEvent.mock.calls[1]![0] as Event).type).toBe('committed');
    expect(onLagged).not.toHaveBeenCalled();
    expect(s.events).toHaveLength(3);
    s.close();
  });

  it('routes lagged to onLagged only, and ignores unknown/garbage frames', () => {
    const s = createEventStore({ socketFactory: factory });
    const onEvent = vi.fn();
    const onLagged = vi.fn();
    s.subscribe(onEvent, onLagged);
    s.start();
    last().open();
    last().send({ type: 'lagged', missed: 5, seq: 9 });
    last().send({ type: 'from-the-future' });
    last().onmessage?.({ data: 'not json' });
    expect(onLagged).toHaveBeenCalledTimes(1);
    expect(onEvent).not.toHaveBeenCalled();
    s.close();
  });

  it('bounds the retained events', () => {
    const s = createEventStore({ socketFactory: factory, maxEvents: 2 });
    s.start();
    last().open();
    for (let i = 1; i <= 4; i++) last().send({ type: 'received', id: String(i), seq: i });
    expect(s.events.map((e) => (e as { id: string }).id)).toEqual(['3', '4']);
    s.close();
  });

  it('reconnects with capped backoff and signals lagged on reopen', () => {
    const s = createEventStore({
      socketFactory: factory,
      baseDelayMs: 100,
      maxDelayMs: 300,
    });
    const onLagged = vi.fn();
    s.subscribe(vi.fn(), onLagged);
    s.start();
    last().open();
    expect(onLagged).not.toHaveBeenCalled(); // first open is not a gap

    last().drop();
    expect(MockSocket.instances).toHaveLength(1);
    vi.advanceTimersByTime(100);
    expect(MockSocket.instances).toHaveLength(2);
    last().drop(); // fails before opening: backoff doubles
    vi.advanceTimersByTime(199);
    expect(MockSocket.instances).toHaveLength(2);
    vi.advanceTimersByTime(1);
    expect(MockSocket.instances).toHaveLength(3);
    last().drop(); // 400 capped to 300
    vi.advanceTimersByTime(300);
    expect(MockSocket.instances).toHaveLength(4);

    last().open();
    expect(onLagged).toHaveBeenCalledTimes(1);

    last().drop(); // backoff reset after success
    vi.advanceTimersByTime(100);
    expect(MockSocket.instances).toHaveLength(5);
    s.close();
  });

  it('stops reconnecting and closes the socket on close()', () => {
    const s = createEventStore({ socketFactory: factory, baseDelayMs: 100 });
    s.start();
    last().open();
    last().drop();
    s.close();
    vi.advanceTimersByTime(10_000);
    expect(MockSocket.instances).toHaveLength(1);

    const s2 = createEventStore({ socketFactory: factory });
    s2.start();
    const sock = last();
    s2.close();
    expect(sock.closed).toBe(true);
    sock.drop();
    vi.advanceTimersByTime(10_000);
    expect(MockSocket.instances).toHaveLength(2);
  });

  it('unsubscribe stops delivery', () => {
    const s = createEventStore({ socketFactory: factory });
    const onEvent = vi.fn();
    const off = s.subscribe(onEvent, vi.fn());
    s.start();
    last().open();
    off();
    last().send({ type: 'received', id: 'A', seq: 1 });
    expect(onEvent).not.toHaveBeenCalled();
    s.close();
  });
});
