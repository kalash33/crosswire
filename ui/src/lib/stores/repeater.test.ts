import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import {
  createRepeater,
  toJson,
  fromJson,
  newTabId,
  deriveTitle,
  responseView,
  type RepeaterApi,
  type RepeaterTab,
} from './repeater.svelte';
import { ApiError } from '../api/errors';
import { hexPage, HEX_PAGE } from '../editor/hex-window';
import { bytesToLatin1 } from '../editor/encoding';
import type { SendOutcomeDto } from '../api/repeater';

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
    expect(r.titleFor(r.tabs.find((t) => t.id === a)!)).toBe('Tab 1');
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

  it('the rename is in-memory only (not in the draft json)', () => {
    const r = createRepeater({ api: mockApi() });
    const id = r.newTab();
    r.renameTab(id, 'x');
    expect(toJson(r.tabs.find((t) => t.id === id)!)).not.toHaveProperty('title');
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
  it('pages are 64 KiB', () => {
    expect(HEX_PAGE).toBe(65536);
  });

  it('an empty buffer has one empty page', () => {
    expect(hexPage(0, 0)).toEqual({ page: 0, pages: 1, start: 0, end: 0 });
  });

  it('a 2 MB buffer is windowed, never whole', () => {
    const n = 2 * 1024 * 1024 + 5;
    expect(hexPage(n, 0)).toEqual({ page: 0, pages: 33, start: 0, end: 65536 });
    expect(hexPage(n, 32)).toEqual({ page: 32, pages: 33, start: 32 * 65536, end: n });
  });

  it('clamps out-of-range and non-integer pages', () => {
    expect(hexPage(100, -3).page).toBe(0);
    expect(hexPage(100, 9)).toEqual({ page: 0, pages: 1, start: 0, end: 100 });
    expect(hexPage(65536 * 2, 1.7).page).toBe(1);
    expect(hexPage(65536 * 2, Number.NaN).page).toBe(0);
  });
});
