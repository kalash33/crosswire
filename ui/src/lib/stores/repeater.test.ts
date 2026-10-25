import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { createRepeater, toJson, fromJson, newTabId, type RepeaterApi, type RepeaterTab } from './repeater.svelte';
import { ApiError } from '../api/errors';
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
