import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import {
  createRepeater,
  toJson,
  fromJson,
  newTabId,
  deriveTitle,
  responseView,
  hostCaret,
  hostHeader,
  targetFromHost,
  responseRawText,
  NEW_TAB_TEMPLATE,
  HISTORY_MAX,
  HISTORY_REQUEST_MAX,
  DRAFT_BUDGET,
  type RepeaterApi,
  type RepeaterTab,
} from './repeater.svelte';
import { ApiError } from '../api/errors';
import { hexPage, HEX_PAGE } from '../editor/hex-window';
import { bytesToLatin1 } from '../editor/encoding';
import type { SendOutcomeDto } from '../api/repeater';
import { sha256Hex } from '../sha256';
import { DIFF_MAX_BYTES } from '../diff/lines';

const ID_RE = /^[0-9A-HJKMNP-TV-Z]{26}$/;

function outcome(over: Partial<SendOutcomeDto> = {}): SendOutcomeDto {
  return {
    exchange_id: '01H',
    scope_state: 'in',
    changes: [],
    skipped: [],
    cert_override: false,
    ttfb_ms: 3,
    total_ms: 9,
    redirects: [],
    ...over,
  };
}

function mockApi(over: Partial<RepeaterApi> = {}): RepeaterApi {
  return {
    sendRequest: vi.fn().mockResolvedValue(outcome()),
    listTabs: vi.fn().mockResolvedValue([]),
    putTab: vi.fn().mockResolvedValue(undefined),
    deleteTab: vi.fn().mockResolvedValue(undefined),
    getExchange: vi.fn(),
    getBody: vi.fn(),
    getRespBody: vi.fn(),
    ...over,
  };
}

const RAW = new Uint8Array([0x47, 0x45, 0x54, 0x20, 0x2f, 0x0d, 0x0a, 0x80, 0xff, 0x00]);

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe('repeater store', () => {
  it('toJson/fromJson round-trip raw bytes incl. non-ASCII, as a Latin-1 string', () => {
    const tab: RepeaterTab = {
      id: 'X',
      conn: { host: 'a.test', port: 443, tls: true, sni: 's', verify: 'accept-invalid', reuse: true, follow: 'in-scope' },
      mode: 'assisted',
      raw: RAW,
    };
    const json = toJson(tab) as { raw: unknown };
    expect(typeof json.raw).toBe('string');
    expect(json.raw).toBe(bytesToLatin1(RAW));
    const back = fromJson('X', JSON.parse(JSON.stringify(json)));
    expect(back).not.toBeNull();
    expect(Array.from(back!.raw)).toEqual(Array.from(RAW));
    expect(back!.conn).toEqual(tab.conn);
    expect(back!.mode).toBe('assisted');
  });

  it('fromJson rejects garbage', () => {
    expect(fromJson('X', null)).toBeNull();
    expect(fromJson('X', { raw: 5 })).toBeNull();
  });

  it('newTabId makes ULID-shaped ids', () => {
    expect(newTabId()).toMatch(ID_RE);
    expect(newTabId()).not.toBe(newTabId());
  });

  it('newTab/selectTab/closeTab manage state and DELETE the draft', async () => {
    const api = mockApi();
    const r = createRepeater({ api });
    const a = r.newTab({ conn: { host: 'a.test' } });
    const b = r.newTab();
    expect(r.tabs.map((t) => t.id)).toEqual([a, b]);
    expect(r.activeTabId).toBe(b);
    expect(r.tabs[0].conn.host).toBe('a.test');
    r.selectTab(a);
    expect(r.activeTab?.id).toBe(a);
    await r.closeTab(a);
    expect(api.deleteTab).toHaveBeenCalledWith(a);
    expect(r.tabs.map((t) => t.id)).toEqual([b]);
    expect(r.activeTabId).toBe(b);
    // a pending autosave for the closed tab must not resurrect it
    await vi.advanceTimersByTimeAsync(1000);
    expect(api.putTab).not.toHaveBeenCalledWith(a, expect.anything());
  });

  it('send posts a correct SendBody (base64 raw) and stores the outcome', async () => {
    const api = mockApi();
    const r = createRepeater({ api });
    const id = r.newTab({
      conn: { host: 'a.test', port: 8443, tls: true, follow: 'in-scope', reuse: true },
      mode: 'assisted',
      raw: RAW,
    });
    await r.send(id);
    expect(api.sendRequest).toHaveBeenCalledTimes(1);
    const body = (api.sendRequest as ReturnType<typeof vi.fn>).mock.calls[0][0];
    expect(body).toMatchObject({
      host: 'a.test',
      port: 8443,
      tls: true,
      mode: 'assisted',
      follow: 'in-scope',
      reuse: true,
      allow_out_of_scope: false,
    });
    expect(Array.from(Uint8Array.from(atob(body.raw), (c) => c.charCodeAt(0)))).toEqual(Array.from(RAW));
    expect(r.tabs[0].outcome?.exchange_id).toBe('01H');
    expect(r.tabs[0].error).toBeUndefined();
  });

  it('a 403/422 error surfaces on the tab without losing the draft', async () => {
    const api = mockApi({
      sendRequest: vi.fn().mockRejectedValue(new ApiError('out_of_scope', 'out of scope: a.test', 403)),
    });
    const r = createRepeater({ api });
    const id = r.newTab({ conn: { host: 'a.test' }, raw: RAW });
    await r.send(id);
    expect(r.tabs[0].error).toContain('out of scope');
    expect(Array.from(r.tabs[0].raw)).toEqual(Array.from(RAW));
    expect(r.tabs[0].conn.host).toBe('a.test');
    expect(r.tabs[0].sending).toBe(false);
  });

  it('a 422 surfaces on the tab and keeps the draft', async () => {
    const api = mockApi({
      sendRequest: vi.fn().mockRejectedValue(new ApiError('bad_request', 'could not parse request', 422)),
    });
    const r = createRepeater({ api });
    const id = r.newTab({ conn: { host: 'a.test' }, raw: RAW });
    await r.send(id);
    expect(r.tabs[0].error).toContain('could not parse');
    expect(Array.from(r.tabs[0].raw)).toEqual(Array.from(RAW));
  });

  it('a draft-save failure uses saveError and does not mask a send error', async () => {
    const api = mockApi({
      sendRequest: vi.fn().mockRejectedValue(new ApiError('out_of_scope', 'out of scope', 403)),
      putTab: vi.fn().mockRejectedValue(new ApiError('x', 'boom', 500)),
    });
    const r = createRepeater({ api });
    const id = r.newTab({ conn: { host: 'a.test' }, raw: RAW });
    await r.send(id);
    await r.flushDrafts();
    expect(r.tabs[0].error).toBe('out of scope');
    expect(r.tabs[0].saveError).toContain('boom');
    await r.send(id);
    expect(r.tabs[0].saveError).toContain('boom'); // send does not clear saveError
  });

  it('send is ignored while the tab is already sending', async () => {
    let release!: (o: SendOutcomeDto) => void;
    const api = mockApi({ sendRequest: vi.fn().mockReturnValue(new Promise((res) => (release = res))) });
    const r = createRepeater({ api });
    const id = r.newTab({ conn: { host: 'a.test' } });
    const p1 = r.send(id);
    await r.send(id);
    expect(api.sendRequest).toHaveBeenCalledTimes(1);
    release(outcome());
    await p1;
  });

  it('PUTs for one tab never overlap or reorder, and close DELETEs last', async () => {
    const log: string[] = [];
    let inflight = 0;
    const api = mockApi({
      putTab: vi.fn().mockImplementation(async (_id: string, json: { raw: string }) => {
        inflight++;
        expect(inflight).toBe(1);
        await new Promise((res) => setTimeout(res, 300));
        log.push('put:' + json.raw.length);
        inflight--;
      }),
      deleteTab: vi.fn().mockImplementation(async () => {
        log.push('delete');
      }),
    });
    const r = createRepeater({ api, debounceMs: 100 });
    const id = r.newTab();
    r.setRaw(id, new Uint8Array(1));
    await vi.advanceTimersByTimeAsync(150); // PUT #1 in flight
    r.setRaw(id, new Uint8Array(2));
    const flush = r.flushDrafts(); // queues PUT #2 behind #1
    const close = r.closeTab(id); // queued PUT is skipped; DELETE after in-flight
    await vi.advanceTimersByTimeAsync(2000);
    await Promise.all([flush, close]);
    expect(log).toEqual(['put:1', 'delete']);
  });

  it('allowOutOfScope option is forwarded', async () => {
    const api = mockApi();
    const r = createRepeater({ api });
    const id = r.newTab({ conn: { host: 'a.test' }, raw: RAW });
    await r.send(id, { allowOutOfScope: true });
    const body = (api.sendRequest as ReturnType<typeof vi.fn>).mock.calls[0][0];
    expect(body.allow_out_of_scope).toBe(true);
  });

  it('rapid edits coalesce into a single debounced PUT', async () => {
    const api = mockApi();
    const r = createRepeater({ api, debounceMs: 500 });
    const id = r.newTab({ conn: { host: 'a.test' } });
    await vi.advanceTimersByTimeAsync(600); // flush newTab's own save
    (api.putTab as ReturnType<typeof vi.fn>).mockClear();

    r.setRaw(id, new Uint8Array([1]));
    await vi.advanceTimersByTimeAsync(200);
    r.setConn(id, { port: 81 });
    await vi.advanceTimersByTimeAsync(200);
    r.setMode(id, 'assisted');
    await vi.advanceTimersByTimeAsync(499);
    expect(api.putTab).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(2);
    expect(api.putTab).toHaveBeenCalledTimes(1);
    const [putId, json] = (api.putTab as ReturnType<typeof vi.fn>).mock.calls[0];
    expect(putId).toBe(id);
    expect(json).toMatchObject({ mode: 'assisted', conn: { port: 81 } });
    expect(json.raw).toBe('\u0001');
  });

  it('flushDrafts saves pending edits immediately', async () => {
    const api = mockApi();
    const r = createRepeater({ api });
    const id = r.newTab();
    await r.flushDrafts();
    expect(api.putTab).toHaveBeenCalledTimes(1);
    expect((api.putTab as ReturnType<typeof vi.fn>).mock.calls[0][0]).toBe(id);
    await vi.advanceTimersByTimeAsync(1000);
    expect(api.putTab).toHaveBeenCalledTimes(1);
  });

  it('loadTabs hydrates from GET /repeater/tabs', async () => {
    const t1 = newTabId();
    const t2 = newTabId();
    const api = mockApi({
      listTabs: vi.fn().mockResolvedValue([
        { tab_id: t2, updated_at: 20, json: toJson({ id: t2, conn: { host: 'b', port: 80, tls: false, reuse: false, follow: 'off' }, mode: 'raw', raw: RAW }) },
        { tab_id: t1, updated_at: 10, json: toJson({ id: t1, conn: { host: 'a', port: 80, tls: false, reuse: false, follow: 'off' }, mode: 'raw', raw: RAW }) },
        { tab_id: 'bad', updated_at: 5, json: 'nope' },
      ]),
    });
    const r = createRepeater({ api });
    await r.loadTabs();
    expect(r.tabs.map((t) => t.id)).toEqual([t1, t2]);
    expect(r.activeTabId).toBe(t1);
    expect(Array.from(r.tabs[0].raw)).toEqual(Array.from(RAW));
    await vi.advanceTimersByTimeAsync(1000);
    expect(api.putTab).not.toHaveBeenCalled(); // hydration is not an edit
  });

  it('sendFromExchange assembles head+body bytes and conn from the exchange', async () => {
    const head = new TextEncoder().encode('POST /x HTTP/1.1\r\nHost: h.test\r\n\r\n');
    const body = new Uint8Array([0xde, 0xad]);
    const api = mockApi({
      getExchange: vi.fn().mockResolvedValue({
        id: 'E1',
        request: {
          head: { type: 'http1-raw', bytes: btoa(bytesToLatin1(head)) },
          connection: { target_addr: 'h.test', port: 8443, sni: 'sni.test', tls_version: 'TLSv1.3' },
        },
      }),
      getBody: vi.fn().mockResolvedValue(body),
    });
    const r = createRepeater({ api });
    const id = await r.sendFromExchange('E1');
    const tab = r.tabs.find((t) => t.id === id)!;
    expect(tab.conn).toMatchObject({ host: 'h.test', port: 8443, tls: true, sni: 'sni.test' });
    expect(Array.from(tab.raw)).toEqual([...head, ...body]);
    expect(r.activeTabId).toBe(id);
  });
});

const enc = (s: string) => latin1(s);
function latin1(s: string): Uint8Array {
  const out = new Uint8Array(s.length);
  for (let i = 0; i < s.length; i++) out[i] = s.charCodeAt(i) & 0xff;
  return out;
}

describe('tab titles', () => {
  it('derives METHOD host from the request line and Host header', () => {
    expect(deriveTitle(enc('GET / HTTP/1.1\r\nHost: example.com\r\n\r\n'), 1)).toBe('GET example.com');
    // LF-only heads, header-name case and surrounding whitespace.
    expect(deriveTitle(enc('POST /x HTTP/1.1\nhOsT:   api.test:8443  \n\n'), 1)).toBe('POST api.test:8443');
  });

  it('falls back to the method when there is no Host header', () => {
    expect(deriveTitle(enc('GET / HTTP/1.1\r\nAccept: */*\r\n\r\n'), 2)).toBe('GET');
  });

  it('ignores a Host line in the body (after the blank line)', () => {
    expect(deriveTitle(enc('GET / HTTP/1.1\r\n\r\nHost: body.test\r\n'), 1)).toBe('GET');
  });

  it('falls back to Tab N for an empty or unparseable draft', () => {
    expect(deriveTitle(new Uint8Array(), 3)).toBe('Tab 3');
    expect(deriveTitle(enc('\r\n\r\n'), 4)).toBe('Tab 4');
  });

  it('bounds a hostile Host value and keeps it as plain text', () => {
    const t = deriveTitle(enc(`GET / HTTP/1.1\r\nHost: <img src=x onerror=alert(1)>${'a'.repeat(500)}\r\n\r\n`), 1);
    expect(t.startsWith('GET <img src=x onerror=alert(1)>')).toBe(true);
    expect(t.length).toBeLessThanOrEqual(80);
  });

  it('titleFor uses the draft and the tab position', () => {
    const r = createRepeater({ api: mockApi() });
    const a = r.newTab();
    const b = r.newTab({ raw: enc('PUT /a HTTP/1.1\r\nHost: h.test\r\n\r\n') });
    // A fresh tab holds the template, whose Host header is still empty.
    expect(r.titleFor(r.tabs.find((t) => t.id === a)!)).toBe('GET');
    expect(r.titleFor(r.tabs.find((t) => t.id === b)!)).toBe('PUT h.test');
  });

  it('an explicit rename sticks across draft edits; an empty rename reverts', () => {
    const r = createRepeater({ api: mockApi() });
    const id = r.newTab({ raw: enc('GET / HTTP/1.1\r\nHost: a.test\r\n\r\n') });
    const tab = () => r.tabs.find((t) => t.id === id)!;
    r.renameTab(id, '  login probe  ');
    expect(r.titleFor(tab())).toBe('login probe');
    r.setRaw(id, enc('POST / HTTP/1.1\r\nHost: b.test\r\n\r\n'));
    expect(r.titleFor(tab())).toBe('login probe');
    r.renameTab(id, '   ');
    expect(r.titleFor(tab())).toBe('POST b.test');
  });

  it('a rename is persisted in the draft and restored on load', async () => {
    const api = mockApi();
    const r = createRepeater({ api });
    const id = r.newTab();
    await r.flushDrafts();
    (api.putTab as ReturnType<typeof vi.fn>).mockClear();
    r.renameTab(id, 'login probe');
    await vi.advanceTimersByTimeAsync(1000);
    expect(api.putTab).toHaveBeenCalledTimes(1);
    const json = (api.putTab as ReturnType<typeof vi.fn>).mock.calls[0][1];
    expect(json).toMatchObject({ title: 'login probe' });
    const back = fromJson(id, JSON.parse(JSON.stringify(json)))!;
    expect(back.title).toBe('login probe');
    // A revert drops the field.
    r.renameTab(id, '');
    expect(toJson(r.tabs[0])).not.toHaveProperty('title');
  });

  it('fromJson bounds a stored title and ignores a non-string one', () => {
    const base = { raw: 'GET / HTTP/1.1\r\n\r\n' };
    expect(fromJson('X', { ...base, title: 'x'.repeat(500) })!.title).toHaveLength(120);
    expect(fromJson('X', { ...base, title: 7 })!.title).toBeUndefined();
    expect(fromJson('X', { ...base, title: '   ' })!.title).toBeUndefined();
  });
});

describe('response pane state', () => {
  const base: RepeaterTab = {
    id: 'T',
    conn: { host: '', port: 80, tls: false, reuse: false, follow: 'off' },
    mode: 'raw',
    raw: new Uint8Array(),
  };

  it('is empty before the first send', () => {
    expect(responseView(base)).toBe('empty');
  });

  it('is sending while a send is in flight with no prior result', () => {
    expect(responseView({ ...base, sending: true })).toBe('sending');
  });

  it('shows the error when the send failed and there is no result', () => {
    expect(responseView({ ...base, error: 'boom' })).toBe('error');
  });

  it('shows the result after a send (kept while re-sending)', async () => {
    const r = createRepeater({ api: mockApi() });
    const id = r.newTab();
    const tab = () => r.tabs.find((t) => t.id === id)!;
    expect(responseView(tab())).toBe('empty');
    await r.send(id);
    expect(responseView(tab())).toBe('result');
    expect(responseView({ ...tab(), sending: true })).toBe('result');
  });
});

describe('hex paging', () => {
  it('pages are 32 KiB', () => {
    expect(HEX_PAGE).toBe(32768);
  });

  it('an empty buffer has one empty page', () => {
    expect(hexPage(0, 0)).toEqual({ page: 0, pages: 1, start: 0, end: 0 });
  });

  it('a 2 MB buffer is windowed, never whole', () => {
    const n = 2 * 1024 * 1024 + 5;
    expect(hexPage(n, 0)).toEqual({ page: 0, pages: 65, start: 0, end: 32768 });
    expect(hexPage(n, 64)).toEqual({ page: 64, pages: 65, start: 64 * 32768, end: n });
  });

  it('clamps out-of-range and non-integer pages', () => {
    expect(hexPage(100, -3).page).toBe(0);
    expect(hexPage(100, 9)).toEqual({ page: 0, pages: 1, start: 0, end: 100 });
    expect(hexPage(32768 * 2, 1.7).page).toBe(1);
    expect(hexPage(32768 * 2, Number.NaN).page).toBe(0);
  });
});

describe('new tab template', () => {
  it('a new tab starts from GET / with an empty Host header, focused on Host', () => {
    const r = createRepeater({ api: mockApi() });
    const id = r.newTab();
    const tab = r.tabs.find((t) => t.id === id)!;
    expect(bytesToLatin1(tab.raw)).toBe(NEW_TAB_TEMPLATE);
    expect(NEW_TAB_TEMPLATE).toBe('GET / HTTP/1.1\r\nHost: \r\n\r\n');
    expect(tab.focusHost).toBe(true);
    expect(hostCaret(tab.raw)).toBe('GET / HTTP/1.1\r\nHost: '.length);
  });

  it('a seeded tab (send to repeater) keeps its bytes and does not grab focus', () => {
    const r = createRepeater({ api: mockApi() });
    const id = r.newTab({ raw: RAW });
    const tab = r.tabs.find((t) => t.id === id)!;
    expect(Array.from(tab.raw)).toEqual(Array.from(RAW));
    expect(tab.focusHost).toBeFalsy();
  });

  it('hostCaret finds the end of the Host line, or null', () => {
    expect(hostCaret(enc('POST /x HTTP/1.1\r\nAccept: */*\r\nhost: a.test\r\n\r\n'))).toBe(
      'POST /x HTTP/1.1\r\nAccept: */*\r\nhost: a.test'.length,
    );
    expect(hostCaret(enc('GET / HTTP/1.1\r\n\r\nHost: body\r\n'))).toBeNull();
  });
});

describe('send history', () => {
  const sendN = (n: number) =>
    vi.fn().mockImplementation(async () => outcome({ exchange_id: `EX${n++}`, ttfb_ms: n, total_ms: 2 * n }));

  it('every send appends a record, newest first, with timings, conn and the sent bytes', async () => {
    const api = mockApi({ sendRequest: sendN(1) });
    const r = createRepeater({ api });
    const id = r.newTab({ conn: { host: 'a.test', port: 8443, tls: true }, raw: RAW });
    await r.send(id);
    r.setRaw(id, enc('GET /2 HTTP/1.1\r\n\r\n'));
    await r.send(id);
    const h = r.tabs[0].history!;
    expect(h).toHaveLength(2);
    expect(h[0].exchange_id).toBe('EX2');
    expect(h[1].exchange_id).toBe('EX1');
    expect(h[1]).toMatchObject({ ttfb_ms: 2, total_ms: 4, scope_state: 'in', request_len: RAW.length });
    expect(h[1].conn).toMatchObject({ host: 'a.test', port: 8443, tls: true });
    expect(Array.from(latin1(atob(h[1].request!)))).toEqual(Array.from(RAW));
    expect(h[1].request_sha256).toBe(sha256Hex(RAW));
    expect(Number.isNaN(Date.parse(h[0].at))).toBe(false);
    expect(h[0].truncated).toBeFalsy();
  });

  it('a failed send is recorded with its error and no exchange id', async () => {
    const api = mockApi({ sendRequest: vi.fn().mockRejectedValue(new ApiError('bad_gateway', 'connect refused', 502)) });
    const r = createRepeater({ api });
    const id = r.newTab({ conn: { host: 'a.test' }, raw: RAW });
    await r.send(id);
    const rec = r.tabs[0].history![0];
    expect(rec.error).toBe('connect refused');
    expect(rec.exchange_id).toBeUndefined();
    expect(rec.request).toBeDefined();
  });

  it('keeps at most 50 records', async () => {
    const api = mockApi({ sendRequest: sendN(1) });
    const r = createRepeater({ api });
    const id = r.newTab({ raw: RAW });
    for (let i = 0; i < 53; i++) await r.send(id);
    const h = r.tabs[0].history!;
    expect(h).toHaveLength(HISTORY_MAX);
    expect(HISTORY_MAX).toBe(50);
    expect(h[0].exchange_id).toBe('EX53');
    expect(h[49].exchange_id).toBe('EX4');
  });

  it('a request over 256 KiB keeps only its digest and is marked truncated', async () => {
    const api = mockApi();
    const r = createRepeater({ api });
    const big = new Uint8Array(HISTORY_REQUEST_MAX + 1).fill(0x41);
    const id = r.newTab({ raw: big });
    await r.send(id);
    const rec = r.tabs[0].history![0];
    expect(rec.truncated).toBe(true);
    expect(rec.request).toBeUndefined();
    expect(rec.request_sha256).toBe(sha256Hex(big));
    expect(rec.request_len).toBe(big.length);
  });

  it('the history is persisted through the draft PUT and loaded back', async () => {
    const api = mockApi({ sendRequest: sendN(1) });
    const r = createRepeater({ api });
    const id = r.newTab({ raw: RAW });
    await r.send(id);
    await r.flushDrafts();
    const json = (api.putTab as ReturnType<typeof vi.fn>).mock.calls.at(-1)![1];
    const back = fromJson(id, JSON.parse(JSON.stringify(json)))!;
    expect(back.history).toHaveLength(1);
    expect(back.history![0]).toEqual(JSON.parse(JSON.stringify(r.tabs[0].history![0])));
  });

  it('fromJson drops malformed records and caps the list', () => {
    const good = { at: '2026-11-01T00:00:00.000Z', request_sha256: 'ab', request_len: 1, conn: { host: 'h', port: 1, tls: false, reuse: false, follow: 'off' }, mode: 'raw' };
    const back = fromJson('X', {
      raw: '',
      history: [
        good,
        null,
        { at: 5 },
        { ...good, at: 'x1', status: 'x', exchange_id: 9 },
        ...Array.from({ length: 60 }, (_, i) => ({ ...good, at: `t${i}` })),
      ],
    })!;
    expect(back.history).toHaveLength(50);
    expect(back.history![1].status).toBeUndefined();
    expect(back.history![1].exchange_id).toBeUndefined();
    expect(fromJson('X', { raw: '', history: 'nope' })!.history).toEqual([]);
    // A stored duplicate (same time, digest and exchange) is dropped: it is the list key.
    expect(fromJson('X', { raw: '', history: [good, good, { ...good, exchange_id: 'E' }] })!.history).toHaveLength(2);
  });

  it('keeps the draft under the server limit by dropping the oldest request bytes first', async () => {
    const api = mockApi({ sendRequest: sendN(1) });
    const r = createRepeater({ api });
    const id = r.newTab({ raw: new Uint8Array(200 * 1024).fill(0x42) });
    for (let i = 0; i < 6; i++) await r.send(id);
    await r.flushDrafts();
    const json = (api.putTab as ReturnType<typeof vi.fn>).mock.calls.at(-1)![1];
    const size = new TextEncoder().encode(JSON.stringify(json)).length;
    expect(size).toBeLessThanOrEqual(DRAFT_BUDGET);
    expect(DRAFT_BUDGET).toBeLessThan(1024 * 1024);
    const h = r.tabs[0].history!;
    expect(h[0].request).toBeDefined(); // the newest keeps its bytes
    expect(h[5].truncated).toBe(true); // the oldest lost them
    expect(h[5].request_sha256).toBe(h[0].request_sha256);
  });

  it('noteResponse fills status and size on the matching record and persists it', async () => {
    const api = mockApi({ sendRequest: sendN(1) });
    const r = createRepeater({ api });
    const id = r.newTab({ raw: RAW });
    await r.send(id);
    await r.flushDrafts();
    (api.putTab as ReturnType<typeof vi.fn>).mockClear();
    r.noteResponse(id, 'EX1', { status: 404, size: 1234 });
    expect(r.tabs[0].history![0]).toMatchObject({ status: 404, size: 1234 });
    await vi.advanceTimersByTimeAsync(1000);
    expect(api.putTab).toHaveBeenCalledTimes(1);
    // Same values again: no new save.
    r.noteResponse(id, 'EX1', { status: 404, size: 1234 });
    await vi.advanceTimersByTimeAsync(1000);
    expect(api.putTab).toHaveBeenCalledTimes(1);
  });

  it('clearHistory empties the list and saves', async () => {
    const api = mockApi();
    const r = createRepeater({ api });
    const id = r.newTab({ raw: RAW });
    await r.send(id);
    r.clearHistory(id);
    expect(r.tabs[0].history).toEqual([]);
    await r.flushDrafts();
    expect((api.putTab as ReturnType<typeof vi.fn>).mock.calls.at(-1)![1]).toMatchObject({ history: [] });
  });
});

describe('restore from history (Review Focus 3)', () => {
  it('restores the exact sent bytes over an edited draft, sets dirty and sends nothing', async () => {
    const api = mockApi({
      sendRequest: vi
        .fn()
        .mockResolvedValueOnce(outcome({ exchange_id: 'A', ttfb_ms: 5, total_ms: 6 }))
        .mockResolvedValueOnce(outcome({ exchange_id: 'B' }))
        .mockResolvedValueOnce(outcome({ exchange_id: 'C' })),
    });
    const r = createRepeater({ api });
    const id = r.newTab({ conn: { host: 'a.test', port: 81 }, raw: RAW });
    await r.send(id);
    r.setRaw(id, enc('POST /edited HTTP/1.1\r\n\r\n'));
    r.setConn(id, { host: 'b.test' });
    await r.send(id);
    r.setRaw(id, enc('PUT /later-edit HTTP/1.1\r\n\r\n'));
    const tab = r.tabs[0];
    const rev = tab.rev ?? 0;
    expect(tab.dirty).toBe(true);
    await r.send(id); // third send so dirty resets, then edit again
    expect(tab.dirty).toBe(false);
    r.setRaw(id, enc('PUT /unsent HTTP/1.1\r\n\r\n'));
    (api.sendRequest as ReturnType<typeof vi.fn>).mockClear();

    r.restore(id, 2); // the first send (A)
    expect(Array.from(tab.raw)).toEqual(Array.from(RAW));
    expect(tab.conn.host).toBe('a.test');
    expect(tab.dirty).toBe(true);
    expect(tab.rev).toBe(rev + 1); // the editor re-seeds
    expect(tab.outcome?.exchange_id).toBe('A');
    expect(tab.outcome?.ttfb_ms).toBe(5);
    expect(tab.notice).toMatch(/not sent/i);
    await vi.advanceTimersByTimeAsync(5000);
    expect(api.sendRequest).not.toHaveBeenCalled();
    // The restore is an edit of the draft: it autosaves.
    const json = (api.putTab as ReturnType<typeof vi.fn>).mock.calls.at(-1)![1] as { raw: string };
    expect(json.raw).toBe(bytesToLatin1(RAW));
  });

  it('a truncated record restores the response only and says so', async () => {
    const api = mockApi({ sendRequest: vi.fn().mockResolvedValue(outcome({ exchange_id: 'BIG' })) });
    const r = createRepeater({ api });
    const big = new Uint8Array(HISTORY_REQUEST_MAX + 10).fill(0x41);
    const id = r.newTab({ raw: big });
    await r.send(id);
    r.setRaw(id, RAW);
    r.restore(id, 0);
    const tab = r.tabs[0];
    expect(Array.from(tab.raw)).toEqual(Array.from(RAW)); // editor untouched
    expect(tab.outcome?.exchange_id).toBe('BIG');
    expect(tab.notice).toMatch(/response only/i);
    expect(api.sendRequest).toHaveBeenCalledTimes(1);
  });

  it('a failed record restores the request and shows its error', async () => {
    const api = mockApi({ sendRequest: vi.fn().mockRejectedValue(new ApiError('x', 'refused', 502)) });
    const r = createRepeater({ api });
    const id = r.newTab({ raw: RAW });
    await r.send(id);
    r.setRaw(id, enc('X'));
    r.restore(id, 0);
    const tab = r.tabs[0];
    expect(Array.from(tab.raw)).toEqual(Array.from(RAW));
    expect(tab.outcome).toBeUndefined();
    expect(tab.error).toBe('refused');
  });

  it('an out-of-range index is ignored', () => {
    const r = createRepeater({ api: mockApi() });
    const id = r.newTab({ raw: RAW });
    r.restore(id, 3);
    expect(Array.from(r.tabs[0].raw)).toEqual(Array.from(RAW));
  });
});

describe('compare with previous', () => {
  function ex(head: string) {
    return { id: 'x', request: {}, response: { head: { type: 'http1-raw', bytes: btoa(head) } } };
  }

  it('picks the shown response and the previous one with a response', async () => {
    const ids = ['A', 'B', 'C'];
    const api = mockApi({
      sendRequest: vi
        .fn()
        .mockResolvedValueOnce(outcome({ exchange_id: ids[0] }))
        .mockRejectedValueOnce(new ApiError('x', 'fail', 502))
        .mockResolvedValueOnce(outcome({ exchange_id: ids[2] })),
    });
    const r = createRepeater({ api });
    const id = r.newTab({ raw: RAW });
    expect(r.comparePair(r.tabs[0])).toBeNull();
    await r.send(id);
    expect(r.comparePair(r.tabs[0])).toBeNull(); // one response only
    await r.send(id); // failed: no response
    await r.send(id);
    const pair = r.comparePair(r.tabs[0])!;
    expect(pair.newer.exchange_id).toBe('C');
    expect(pair.older.exchange_id).toBe('A');
  });

  it('loads both responses as Raw text (head + body, Latin-1)', async () => {
    const api = mockApi({
      sendRequest: vi
        .fn()
        .mockResolvedValueOnce(outcome({ exchange_id: 'A' }))
        .mockResolvedValueOnce(outcome({ exchange_id: 'B' })),
      getExchange: vi.fn().mockImplementation(async (x: string) =>
        ex(x === 'A' ? 'HTTP/1.1 200 OK\r\nX: 1\r\n\r\n' : 'HTTP/1.1 500 Oops\r\nX: 1\r\n\r\n'),
      ),
      getRespBody: vi.fn().mockImplementation(async (x: string) => {
        if (x === 'B') throw new ApiError('not_found', 'no body', 404);
        return new Uint8Array([0x68, 0xe9, 0x0a, 0x62]);
      }),
    });
    const r = createRepeater({ api });
    const id = r.newTab({ raw: RAW });
    await r.send(id);
    await r.send(id);
    const c = (await r.compareWithPrevious(id))!;
    expect(c.older.exchange_id).toBe('A');
    expect(c.olderText).toBe('HTTP/1.1 200 OK\r\nX: 1\r\n\r\nhé\nb');
    expect(c.newerText).toBe('HTTP/1.1 500 Oops\r\nX: 1\r\n\r\n');
  });

  it('responseRawText renders a normalized head as name: value lines', () => {
    const t = responseRawText(
      { response: { head: { type: 'normalized', map: [[':status', btoa('204')], ['a', btoa('b')]] } } },
      new Uint8Array(),
    );
    expect(t).toBe(':status: 204\r\na: b\r\n\r\n');
    expect(responseRawText({}, new Uint8Array([0x7a]))).toBe('z');
  });
});

describe('target from the Host header', () => {
  it('hostHeader reads the head only', () => {
    expect(hostHeader(enc('GET / HTTP/1.1\r\nHOST:  a.test:8443 \r\n\r\n'))).toBe('a.test:8443');
    expect(hostHeader(enc('GET / HTTP/1.1\r\n\r\nHost: body.test\r\n'))).toBeNull();
    expect(hostHeader(enc('GET / HTTP/1.1\r\nHost: \r\n\r\n'))).toBe('');
  });

  it('targetFromHost parses host, port and bracketed IPv6', () => {
    const t = (s: string) => targetFromHost(enc(`GET / HTTP/1.1\r\nHost: ${s}\r\n\r\n`));
    expect(t('api.test')).toEqual({ host: 'api.test' });
    expect(t('api.test:443')).toEqual({ host: 'api.test', port: 443 });
    expect(t('[::1]:8080')).toEqual({ host: '::1', port: 8080 });
    expect(t('[2001:db8::2]')).toEqual({ host: '2001:db8::2' });
    expect(t('a.test:0')).toBeNull();
    expect(t('a.test:65536')).toBeNull();
    expect(t('a b')).toBeNull();
    expect(t('')).toBeNull();
    expect(t('a.test:')).toBeNull();
    expect(targetFromHost(enc('GET / HTTP/1.1\r\n\r\n'))).toBeNull();
  });

  it('send fills an empty target from Host:port 443 and turns TLS on', async () => {
    const api = mockApi();
    const r = createRepeater({ api });
    const id = r.newTab();
    r.setRaw(id, enc('GET / HTTP/1.1\r\nHost: api.example.test:443\r\n\r\n'));
    await r.send(id);
    const body = (api.sendRequest as ReturnType<typeof vi.fn>).mock.calls[0][0];
    expect(body).toMatchObject({ host: 'api.example.test', port: 443, tls: true });
    expect(r.tabs[0].conn).toMatchObject({ host: 'api.example.test', port: 443, tls: true });
    await r.flushDrafts();
    expect((api.putTab as ReturnType<typeof vi.fn>).mock.calls.at(-1)![1]).toMatchObject({
      conn: { host: 'api.example.test', port: 443, tls: true },
    });
  });

  it('a bare Host keeps port 80 without TLS; a non-standard port keeps TLS as set', async () => {
    const api = mockApi();
    const r = createRepeater({ api });
    const a = r.newTab({ raw: enc('GET / HTTP/1.1\r\nHost: plain.test\r\n\r\n') });
    await r.send(a);
    expect((api.sendRequest as ReturnType<typeof vi.fn>).mock.calls[0][0]).toMatchObject({
      host: 'plain.test',
      port: 80,
      tls: false,
    });
    const b = r.newTab({ conn: { tls: true }, raw: enc('GET / HTTP/1.1\r\nHost: odd.test:8443\r\n\r\n') });
    await r.send(b);
    expect((api.sendRequest as ReturnType<typeof vi.fn>).mock.calls[1][0]).toMatchObject({
      host: 'odd.test',
      port: 8443,
      tls: true,
    });
  });

  it('never overwrites a target the user set', async () => {
    const api = mockApi();
    const r = createRepeater({ api });
    const id = r.newTab({ conn: { host: 'set.test', port: 9000 }, raw: enc('GET / HTTP/1.1\r\nHost: other.test:443\r\n\r\n') });
    await r.send(id);
    expect((api.sendRequest as ReturnType<typeof vi.fn>).mock.calls[0][0]).toMatchObject({ host: 'set.test', port: 9000, tls: false });
    expect(r.tabs[0].conn.host).toBe('set.test');
  });

  it('a blank or garbage Host leaves the target empty for the server to refuse', async () => {
    const api = mockApi();
    const r = createRepeater({ api });
    const a = r.newTab();
    await r.send(a);
    const b = r.newTab({ raw: enc('GET / HTTP/1.1\r\nHost: bad host:99999\r\n\r\n') });
    await r.send(b);
    const calls = (api.sendRequest as ReturnType<typeof vi.fn>).mock.calls;
    expect(calls[0][0].host).toBe('');
    expect(calls[1][0].host).toBe('');
  });
});

describe('compare size cap', () => {
  it('a side over DIFF_MAX_BYTES is reported too large without decoding it', async () => {
    const huge = new Uint8Array(DIFF_MAX_BYTES + 1).fill(0x61);
    const api = mockApi({
      sendRequest: vi
        .fn()
        .mockResolvedValueOnce(outcome({ exchange_id: 'A' }))
        .mockResolvedValueOnce(outcome({ exchange_id: 'B' })),
      getExchange: vi.fn().mockResolvedValue({ response: { head: { type: 'http1-raw', bytes: btoa('HTTP/1.1 200 OK\r\n\r\n') } } }),
      getRespBody: vi.fn().mockImplementation(async (x: string) => (x === 'B' ? huge : new Uint8Array([0x61]))),
    });
    const r = createRepeater({ api });
    const id = r.newTab({ raw: RAW });
    await r.send(id);
    await r.send(id);
    const c = (await r.compareWithPrevious(id))!;
    expect(c.tooLarge).toBe(true);
    expect(c.newerText).toBe('');
    expect(c.olderText).toBe('');
  });

  it('records the request line for the compare labels', async () => {
    const r = createRepeater({ api: mockApi() });
    const id = r.newTab({ raw: enc('POST /api/login?x=1 HTTP/1.1\r\nHost: a\r\n\r\n') });
    await r.send(id);
    expect(r.tabs[0].history![0].target).toBe('POST /api/login?x=1');
  });
});
