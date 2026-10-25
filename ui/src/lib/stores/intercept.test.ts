import { describe, it, expect, vi } from 'vitest';
import { createIntercept, splitRequest, type InterceptApi } from './intercept.svelte';
import { ApiError } from '../api/errors';
import type { InterceptStatus, HeldDto, RulesDto } from '../api/intercept';
import { latin1ToBytes, bytesToLatin1 } from '../editor/encoding';

const RULES: RulesDto = {
  skip_extensions: ['png'],
  skip_out_of_scope: true,
  overflow: 'forward-newest',
  on_timeout: 'forward',
};
const H1: HeldDto = { id: 'A', host: 'a.test', method: 'GET', path: '/', size: 10, editable: true };
const H2: HeldDto = { id: 'B', host: 'b.test', method: 'POST', path: '/x', size: 99, editable: false };

function status(over: Partial<InterceptStatus> = {}): InterceptStatus {
  return { enabled: false, rules: RULES, held: [H1, H2], ...over };
}

function mockApi(over: Partial<InterceptApi> = {}): InterceptApi {
  return {
    getStatus: vi.fn().mockResolvedValue(status()),
    putEnabled: vi.fn().mockResolvedValue(undefined),
    putRules: vi.fn().mockResolvedValue(undefined),
    postForward: vi.fn().mockResolvedValue(undefined),
    postDrop: vi.fn().mockResolvedValue(undefined),
    postForwardAll: vi.fn().mockResolvedValue(undefined),
    getHeadBytes: vi.fn().mockResolvedValue(new Uint8Array([1])),
    getBodyBytes: vi.fn().mockResolvedValue(null),
    ...over,
  };
}

const unb64 = (s: string) => Uint8Array.from(atob(s), (c) => c.charCodeAt(0));

describe('intercept store', () => {
  it('refresh loads enabled/rules/held (oldest first) and exposes editable', async () => {
    const i = createIntercept({ api: mockApi() });
    await i.refresh();
    expect(i.enabled).toBe(false);
    expect(i.rules).toEqual(RULES);
    expect(i.held.map((h) => h.id)).toEqual(['A', 'B']);
    expect(i.held[0].editable).toBe(true);
    expect(i.held[1].editable).toBe(false);
  });

  it('setEnabled PUTs {enabled} and reflects it', async () => {
    const api = mockApi();
    const i = createIntercept({ api });
    await i.setEnabled(true);
    expect(api.putEnabled).toHaveBeenCalledWith(true);
    expect(i.enabled).toBe(true);
  });

  it('forward with edited bytes base64-encodes head and body (round-trips)', async () => {
    const api = mockApi();
    const i = createIntercept({ api });
    const head = new Uint8Array([0x47, 0x45, 0x54, 0x0d, 0x0a, 0x0d, 0x0a, 0x80, 0xff]);
    const body = new Uint8Array([0, 1, 2, 0xfe, 0xff]);
    await i.forward('A', { head, body });
    const [id, fb] = (api.postForward as ReturnType<typeof vi.fn>).mock.calls[0];
    expect(id).toBe('A');
    expect(Array.from(unb64(fb.head))).toEqual(Array.from(head));
    expect(Array.from(unb64(fb.body))).toEqual(Array.from(body));
  });

  it('forward with an edited head only sends no body key', async () => {
    const api = mockApi();
    const i = createIntercept({ api });
    await i.forward('A', { head: latin1ToBytes('X') });
    const fb = (api.postForward as ReturnType<typeof vi.fn>).mock.calls[0][1];
    expect(fb.head).toBe(btoa('X'));
    expect('body' in fb).toBe(false);
  });

  it('forward without edits posts no body (original bytes)', async () => {
    const api = mockApi();
    const i = createIntercept({ api });
    await i.forward('A');
    expect(api.postForward).toHaveBeenCalledWith('A', undefined);
  });

  it('drop / forwardAll / putRules hit the right wrappers', async () => {
    const api = mockApi();
    const i = createIntercept({ api });
    await i.drop('A');
    await i.forwardAll();
    const r = { ...RULES, on_timeout: 'respond-504' as const };
    await i.putRules(r);
    expect(api.postDrop).toHaveBeenCalledWith('A');
    expect(api.postForwardAll).toHaveBeenCalled();
    expect(api.putRules).toHaveBeenCalledWith(r);
    expect(i.rules).toEqual(r);
  });

  it('an intercept event refreshes; lagged refreshes', async () => {
    const api = mockApi();
    let onEvent: (e: unknown) => void = () => {};
    let onLagged: () => void = () => {};
    const i = createIntercept({
      api,
      subscribe: (a, b) => {
        onEvent = a as (e: unknown) => void;
        onLagged = b;
        return () => {};
      },
    });
    i.start();
    expect(api.getStatus).toHaveBeenCalledTimes(1);
    onEvent({ type: 'intercept', held: 2 });
    expect(api.getStatus).toHaveBeenCalledTimes(2);
    onEvent({ type: 'committed', ids: [], seq: 1 });
    expect(api.getStatus).toHaveBeenCalledTimes(2);
    onLagged();
    expect(api.getStatus).toHaveBeenCalledTimes(3);
  });

  it('heldBytes yields body:null on a 404 body', async () => {
    const i = createIntercept({ api: mockApi() });
    expect(await i.heldBytes('A')).toEqual({ head: new Uint8Array([1]), body: null });
  });

  it('heldBytes returns head and body', async () => {
    const api = mockApi({ getBodyBytes: vi.fn().mockResolvedValue(new Uint8Array([9])) });
    const r = await createIntercept({ api }).heldBytes('A');
    expect(Array.from(r.body!)).toEqual([9]);
  });

  it('heldBytes propagates non-404 errors', async () => {
    const api = mockApi({ getHeadBytes: vi.fn().mockRejectedValue(new ApiError('x', 'boom', 500)) });
    await expect(createIntercept({ api }).heldBytes('A')).rejects.toThrow('boom');
  });
});

describe('splitRequest', () => {
  it('splits after the first CRLFCRLF', () => {
    const { head, body } = splitRequest(latin1ToBytes('POST / HTTP/1.1\r\nA: b\r\n\r\nBODY\r\n\r\nX'));
    expect(bytesToLatin1(head)).toBe('POST / HTTP/1.1\r\nA: b\r\n\r\n');
    expect(bytesToLatin1(body)).toBe('BODY\r\n\r\nX');
  });
  it('no terminator: all head, empty body', () => {
    const { head, body } = splitRequest(latin1ToBytes('GET /'));
    expect(bytesToLatin1(head)).toBe('GET /');
    expect(body.length).toBe(0);
  });
});
