// Repeater store: tabs, debounced draft autosave, send, and "send to repeater".
//
// Wire notes (from crates/crosswire-api/src/routes/repeater.rs):
//  - Tab ids are ULIDs (the server parse_ulid()s the path id), so newTabId()
//    mints a real ULID client-side.
//  - POST /repeater/send takes `raw` as STANDARD BASE64 of the request bytes.
//  - A tab draft's `json` is opaque to the server (<= 1 MiB). We store `raw`
//    there as a Latin-1 string (encoding.ts, 1:1 byte<->char), never a JS array.
import { apiGetBytes, apiGet } from '../api/client';
import { ApiError } from '../api/errors';
import { toasts } from '../ui/toasts.svelte';
import {
  deleteTab,
  listTabs,
  putTab,
  sendRequest,
  type DraftDto,
  type SendOutcomeDto,
} from '../api/repeater';
import type { SendBody } from '../api/types';
import { bytesToLatin1, latin1ToBytes } from '../editor/encoding';

export type Follow = 'off' | 'in-scope' | 'always';
export type Mode = 'assisted' | 'raw';

export interface TabConn {
  host: string;
  port: number;
  tls: boolean;
  sni?: string;
  verify?: string;
  reuse: boolean;
  follow: Follow;
}

/** The UI model of one repeater tab. `outcome`/`error`/`sending` are not persisted. */
export interface RepeaterTab {
  id: string;
  conn: TabConn;
  mode: Mode;
  /** The full request bytes. */
  raw: Uint8Array;
  outcome?: SendOutcomeDto;
  /** Last send failure (403 out-of-scope / 422 / 502). */
  error?: string;
  /** Last draft-autosave failure; independent of `error`. */
  saveError?: string;
  sending?: boolean;
  /** Explicit user rename (in-memory only, never persisted). Wins over the derived title. */
  title?: string;
}

export interface TabSeed {
  conn?: Partial<TabConn>;
  mode?: Mode;
  raw?: Uint8Array;
}

/** Injectable I/O so tests need no fetch. */
export interface RepeaterApi {
  sendRequest(body: SendBody): Promise<SendOutcomeDto>;
  listTabs(): Promise<DraftDto[]>;
  putTab(id: string, json: unknown): Promise<void>;
  deleteTab(id: string): Promise<void>;
  /** `GET /exchanges/{id}` (the full model `Exchange` JSON). */
  getExchange(id: string): Promise<unknown>;
  /** `GET /exchanges/{id}/body/req` as exact bytes. */
  getBody(id: string): Promise<Uint8Array>;
}

const defaultApi: RepeaterApi = {
  sendRequest,
  listTabs,
  putTab,
  deleteTab,
  getExchange: (id) => apiGet<unknown>(`/api/v1/exchanges/${encodeURIComponent(id)}`),
  getBody: async (id) =>
    (await apiGetBytes(`/api/v1/exchanges/${encodeURIComponent(id)}/body/req`)).bytes,
};

// ---- ULID (Crockford base32: 48-bit ms time + 80 random bits) ----

const CROCKFORD = '0123456789ABCDEFGHJKMNPQRSTVWXYZ';

export function newTabId(): string {
  let t = Date.now();
  let time = '';
  for (let i = 0; i < 10; i++) {
    time = CROCKFORD[t % 32] + time;
    t = Math.floor(t / 32);
  }
  const rnd = new Uint8Array(16);
  crypto.getRandomValues(rnd);
  let rand = '';
  for (let i = 0; i < 16; i++) rand += CROCKFORD[rnd[i] & 31];
  return time + rand;
}

// ---- tab titles ----

/** Only the head is scanned for a title; a huge body is never decoded. */
const TITLE_SCAN_BYTES = 8192;
const TITLE_MAX = 80;
const METHOD_RE = /^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/;

/**
 * `METHOD host` from the draft's request line and Host header, read as
 * Latin-1 (1:1 bytes, no decoding). No Host header gives the method alone;
 * no request line gives `Tab N`. The value is captured data: callers render
 * it as text only. Bounded to TITLE_MAX characters.
 */
export function deriveTitle(raw: Uint8Array, n: number): string {
  const head = bytesToLatin1(raw.subarray(0, TITLE_SCAN_BYTES));
  const lines = head.split(/\r?\n/);
  const method = (lines[0] ?? '').trim().split(/\s+/)[0] ?? '';
  if (!method || !METHOD_RE.test(method)) return `Tab ${n}`;
  let host = '';
  for (let i = 1; i < lines.length; i++) {
    const line = lines[i];
    if (line === '' || line === '\r') break; // end of head
    const colon = line.indexOf(':');
    if (colon > 0 && line.slice(0, colon).trim().toLowerCase() === 'host') {
      host = line.slice(colon + 1).trim();
      break;
    }
  }
  const title = host ? `${method} ${host}` : method;
  return title.length > TITLE_MAX ? `${title.slice(0, TITLE_MAX - 1)}…` : title;
}

// ---- response pane ----

export type ResponseView = 'empty' | 'sending' | 'error' | 'result';

/** What the response pane shows. A previous result stays visible while re-sending. */
export function responseView(tab: RepeaterTab): ResponseView {
  if (tab.outcome) return 'result';
  if (tab.sending) return 'sending';
  if (tab.error) return 'error';
  return 'empty';
}

/** Hex view page size: the pane never renders more than this many bytes. */
export const HEX_PAGE_BYTES = 64 * 1024;

export interface HexWindow {
  page: number;
  pages: number;
  start: number;
  end: number;
}

/** The byte window for `page` of a `length`-byte buffer, clamped to valid pages. */
export function hexWindow(length: number, page: number): HexWindow {
  const len = Math.max(0, Math.floor(length) || 0);
  const pages = Math.max(1, Math.ceil(len / HEX_PAGE_BYTES));
  const p = Math.min(pages - 1, Math.max(0, Math.floor(page) || 0));
  const start = p * HEX_PAGE_BYTES;
  return { page: p, pages, start, end: Math.min(len, start + HEX_PAGE_BYTES) };
}

// ---- tab <-> draft json ----

const FOLLOWS: readonly Follow[] = ['off', 'in-scope', 'always'];

/** Serialize a tab to the draft `json`. `raw` is a Latin-1 string (byte-exact). */
export function toJson(tab: RepeaterTab): unknown {
  return { conn: { ...tab.conn }, mode: tab.mode, raw: bytesToLatin1(tab.raw) };
}

/** Parse a draft `json` back into a tab; null if it is not a draft we wrote. */
export function fromJson(id: string, json: unknown): RepeaterTab | null {
  if (typeof json !== 'object' || json === null) return null;
  const j = json as { conn?: Partial<TabConn>; mode?: unknown; raw?: unknown };
  if (typeof j.raw !== 'string') return null;
  const c = j.conn ?? {};
  const conn: TabConn = {
    host: typeof c.host === 'string' ? c.host : '',
    port: typeof c.port === 'number' ? c.port : 80,
    tls: c.tls === true,
    reuse: c.reuse === true,
    follow: FOLLOWS.includes(c.follow as Follow) ? (c.follow as Follow) : 'off',
  };
  if (typeof c.sni === 'string') conn.sni = c.sni;
  if (typeof c.verify === 'string') conn.verify = c.verify;
  return { id, conn, mode: j.mode === 'assisted' ? 'assisted' : 'raw', raw: latin1ToBytes(j.raw) };
}

function b64(bytes: Uint8Array): string {
  return btoa(bytesToLatin1(bytes));
}

function unb64(s: string): Uint8Array {
  return latin1ToBytes(atob(s));
}

function messageOf(e: unknown): string {
  return e instanceof ApiError || e instanceof Error ? e.message : String(e);
}

export interface RepeaterOptions {
  api?: RepeaterApi;
  /** Autosave debounce in ms (default 500). */
  debounceMs?: number;
}

export function createRepeater(opts: RepeaterOptions = {}) {
  const api = opts.api ?? defaultApi;
  const debounceMs = opts.debounceMs ?? 500;

  let tabs = $state<RepeaterTab[]>([]);
  let activeTabId = $state<string | null>(null);
  const activeTab = $derived(tabs.find((t) => t.id === activeTabId));
  let loadError = $state<string | null>(null);

  // One timer per tab: a new edit replaces it, so rapid edits coalesce into a
  // single PUT fired `debounceMs` after the LAST change. Plain (non-reactive).
  const timers: Record<string, ReturnType<typeof setTimeout> | undefined> = Object.create(null);

  function find(id: string): RepeaterTab | undefined {
    return tabs.find((t) => t.id === id);
  }

  // Per-tab promise chain: a tab's PUTs run strictly in order, each reading the
  // tab's CURRENT state when it runs, so an older in-flight PUT can never land
  // after a newer one. A closed tab is gone from `tabs`, so queued PUTs skip.
  const chains: Record<string, Promise<void> | undefined> = Object.create(null);

  function save(id: string): Promise<void> {
    delete timers[id];
    const run = (chains[id] ?? Promise.resolve()).then(async () => {
      const tab = find(id);
      if (!tab) return;
      try {
        await api.putTab(id, toJson(tab));
        tab.saveError = undefined;
      } catch (e) {
        // The draft stays in memory; the next edit retries.
        tab.saveError = `draft not saved: ${messageOf(e)}`;
      }
    });
    chains[id] = run;
    return run;
  }

  function schedule(id: string): void {
    const prev = timers[id];
    if (prev !== undefined) clearTimeout(prev);
    timers[id] = setTimeout(() => void save(id), debounceMs);
  }

  /** Save every pending draft now (e.g. on tab close / page hide). */
  async function flushDrafts(): Promise<void> {
    const ids = Object.keys(timers);
    for (const id of ids) clearTimeout(timers[id]);
    await Promise.all(ids.map((id) => save(id)));
  }

  function newTab(seed: TabSeed = {}): string {
    const tab: RepeaterTab = {
      id: newTabId(),
      conn: {
        host: '',
        port: 80,
        tls: false,
        reuse: false,
        follow: 'off',
        ...seed.conn,
      },
      mode: seed.mode ?? 'assisted',
      raw: seed.raw ?? new Uint8Array(),
    };
    tabs.push(tab);
    activeTabId = tab.id;
    schedule(tab.id);
    return tab.id;
  }

  function selectTab(id: string): void {
    if (find(id)) activeTabId = id;
  }

  async function closeTab(id: string): Promise<void> {
    const i = tabs.findIndex((t) => t.id === id);
    if (i < 0) return;
    const timer = timers[id];
    if (timer !== undefined) clearTimeout(timer);
    delete timers[id];
    tabs.splice(i, 1); // queued PUTs now skip (tab not found)
    if (activeTabId === id) activeTabId = tabs[Math.min(i, tabs.length - 1)]?.id ?? null;
    try {
      // Let any in-flight PUT land first, then DELETE last.
      await chains[id];
      delete chains[id];
      await api.deleteTab(id);
    } catch (e) {
      loadError = `could not delete draft: ${messageOf(e)}`;
    }
  }

  function setRaw(id: string, raw: Uint8Array): void {
    const t = find(id);
    if (!t) return;
    t.raw = raw;
    schedule(id);
  }

  /** The tab-strip title: an explicit rename, else derived from the draft. */
  function titleFor(tab: RepeaterTab): string {
    if (tab.title) return tab.title;
    const i = tabs.findIndex((t) => t.id === tab.id);
    return deriveTitle(tab.raw, (i < 0 ? tabs.length : i) + 1);
  }

  /** Rename a tab locally (not persisted). A blank name reverts to the derived title. */
  function renameTab(id: string, title: string): void {
    const t = find(id);
    if (!t) return;
    const v = title.trim();
    t.title = v === '' ? undefined : v.slice(0, 120);
  }

  function setConn(id: string, patch: Partial<TabConn>): void {
    const t = find(id);
    if (!t) return;
    Object.assign(t.conn, patch);
    schedule(id);
  }

  function setMode(id: string, mode: Mode): void {
    const t = find(id);
    if (!t) return;
    t.mode = mode;
    schedule(id);
  }

  async function send(id: string, o: { allowOutOfScope?: boolean } = {}): Promise<void> {
    const t = find(id);
    if (!t || t.sending) return;
    const body: SendBody = {
      host: t.conn.host,
      port: t.conn.port,
      tls: t.conn.tls,
      raw: b64(t.raw),
      mode: t.mode,
      follow: t.conn.follow,
      reuse: t.conn.reuse,
      allow_out_of_scope: o.allowOutOfScope === true,
    };
    if (t.conn.sni) body.sni = t.conn.sni;
    if (t.conn.verify) body.verify = t.conn.verify;
    t.sending = true;
    t.error = undefined;
    try {
      const outcome = await api.sendRequest(body);
      const cur = find(id);
      if (cur) cur.outcome = outcome;
    } catch (e) {
      // 403 out-of-scope / 422 unparseable / 502 upstream: show it, keep the draft.
      const cur = find(id);
      if (cur) cur.error = messageOf(e);
      toasts.push({ kind: 'error', text: `Send failed: ${messageOf(e)}` });
    } finally {
      const cur = find(id);
      if (cur) cur.sending = false;
    }
  }

  async function loadTabs(): Promise<void> {
    loadError = null;
    let drafts: DraftDto[];
    try {
      drafts = await api.listTabs();
    } catch (e) {
      loadError = messageOf(e);
      return;
    }
    // Preserve any tab already in memory (e.g. a tab seeded by send-to-repeater
    // just before this view mounted and hydrated). Its debounced draft PUT may
    // not have landed yet, so it would be absent from the GET — merging by id
    // keeps the seed instead of clobbering it.
    const existing = tabs;
    const loaded: RepeaterTab[] = [];
    for (const d of [...drafts].sort((a, b) => a.updated_at - b.updated_at)) {
      const tab = fromJson(d.tab_id, d.json);
      if (tab && !existing.some((e) => e.id === tab.id)) loaded.push(tab);
    }
    // Hydration is not an edit: no autosave is scheduled.
    tabs = [...existing, ...loaded];
    if (!activeTabId || !find(activeTabId)) activeTabId = tabs[0]?.id ?? null;
  }

  /**
   * Seed a new tab from a history exchange (best-effort).
   *
   * Raw bytes = the stored request HEAD bytes (`request.head.bytes`, base64 in
   * `GET /exchanges/{id}`, only for `http1-raw` heads, ending in the blank line)
   * followed by the request body from `GET /exchanges/{id}/body/req`. Conn comes
   * from `request.connection` (`target_addr`, `port`, `sni`); the model records
   * no scheme, so `tls` is inferred from a non-null `tls_version`. A missing body
   * (404) is treated as empty. Throws if the exchange has no raw HTTP/1 head.
   */
  async function sendFromExchange(exchangeId: string): Promise<string> {
    const ex = (await api.getExchange(exchangeId)) as {
      request?: {
        head?: { type?: string; bytes?: string };
        connection?: { target_addr?: string; port?: number; sni?: string | null; tls_version?: string | null };
      };
    };
    const req = ex.request;
    if (req?.head?.type !== 'http1-raw' || typeof req.head.bytes !== 'string') {
      throw new ApiError('unsupported', 'this exchange has no raw HTTP/1 request to repeat', 0);
    }
    const head = unb64(req.head.bytes);
    let body: Uint8Array = new Uint8Array();
    try {
      body = await api.getBody(exchangeId);
    } catch (e) {
      if (!(e instanceof ApiError && e.status === 404)) throw e;
    }
    const raw = new Uint8Array(head.length + body.length);
    raw.set(head, 0);
    raw.set(body, head.length);
    const c = req.connection ?? {};
    const tls = typeof c.tls_version === 'string' && c.tls_version !== '';
    const conn: Partial<TabConn> = {
      host: c.target_addr ?? '',
      port: typeof c.port === 'number' && c.port > 0 ? c.port : tls ? 443 : 80,
      tls,
    };
    if (c.sni) conn.sni = c.sni;
    return newTab({ conn, raw });
  }

  return {
    get tabs() {
      return tabs;
    },
    get activeTabId() {
      return activeTabId;
    },
    get activeTab() {
      return activeTab;
    },
    get loadError() {
      return loadError;
    },
    newTab,
    closeTab,
    selectTab,
    setRaw,
    setConn,
    setMode,
    titleFor,
    renameTab,
    send,
    loadTabs,
    flushDrafts,
    sendFromExchange,
  };
}

export const repeater = createRepeater();
