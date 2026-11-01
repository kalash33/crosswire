// Repeater store: tabs, debounced draft autosave, send, and "send to repeater".
//
// Wire notes (from crates/crosswire-api/src/routes/repeater.rs):
//  - Tab ids are ULIDs (the server parse_ulid()s the path id), so newTabId()
//    mints a real ULID client-side.
//  - POST /repeater/send takes `raw` as STANDARD BASE64 of the request bytes.
//  - A tab draft's `json` is opaque to the server (<= 1 MiB). We store `raw`
//    there as a Latin-1 string (encoding.ts, 1:1 byte<->char), never a JS array.
//    The draft also carries the tab's rename (`title`) and its send history
//    (`history`, newest first); see SendRecord and fitDraft() for the bounds.
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
import { sha256Hex } from '../sha256';
import { isoNow, timeShort } from '../format';

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

/**
 * One send of a tab, as kept in its draft. Every string here except `at` came
 * from the user or the wire: render as text only.
 */
export interface SendRecord {
  /** ISO time the send started. */
  at: string;
  /** Response status, filled in once the response has been read. */
  status?: number;
  ttfb_ms?: number;
  total_ms?: number;
  /** Response body size in bytes, filled in once read. */
  size?: number;
  /** The recorded exchange; absent for a failed send. */
  exchange_id?: string;
  scope_state?: string;
  /** Standard base64 of the sent bytes; absent when `truncated`. */
  request?: string;
  /** Lowercase hex SHA-256 of the sent bytes (always present). */
  request_sha256: string;
  request_len: number;
  /** The bytes were not kept (over HISTORY_REQUEST_MAX, or dropped to fit the draft). */
  truncated?: boolean;
  conn: TabConn;
  mode: Mode;
  /** Why the send failed. */
  error?: string;
}

/** Records kept per tab. */
export const HISTORY_MAX = 50;
/** Largest request whose bytes a record keeps. */
export const HISTORY_REQUEST_MAX = 256 * 1024;
/**
 * The draft size we aim for, measured as UTF-8 JSON. The server refuses
 * drafts over 1 MiB; the margin covers small escaping differences.
 */
export const DRAFT_BUDGET = 960 * 1024;

/** The request a new, unseeded tab starts with. */
export const NEW_TAB_TEMPLATE = 'GET / HTTP/1.1\r\nHost: \r\n\r\n';

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
  /** Explicit user rename, persisted in the draft. Wins over the derived title. */
  title?: string;
  /** Sends of this tab, newest first (persisted). */
  history?: SendRecord[];
  /** The editor holds bytes that have not been sent since the last change. */
  dirty?: boolean;
  /** Bumped when the store replaces `raw` (restore): the editor re-seeds. */
  rev?: number;
  /** A fresh template tab: the editor puts the caret on the Host header once. */
  focusHost?: boolean;
  /** A one-line note about what the panes show (after a restore). */
  notice?: string;
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
  /** `GET /exchanges/{id}/body/resp` as exact bytes. */
  getRespBody(id: string): Promise<Uint8Array>;
}

const defaultApi: RepeaterApi = {
  sendRequest,
  listTabs,
  putTab,
  deleteTab,
  getExchange: (id) => apiGet<unknown>(`/api/v1/exchanges/${encodeURIComponent(id)}`),
  getBody: async (id) =>
    (await apiGetBytes(`/api/v1/exchanges/${encodeURIComponent(id)}/body/req`)).bytes,
  getRespBody: async (id) =>
    (await apiGetBytes(`/api/v1/exchanges/${encodeURIComponent(id)}/body/resp`)).bytes,
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

/**
 * The offset just after the Host header's value in the request head, where a
 * fresh tab puts the caret; null when the head has no Host header.
 */
export function hostCaret(raw: Uint8Array): number | null {
  const head = bytesToLatin1(raw.subarray(0, TITLE_SCAN_BYTES));
  let pos = head.indexOf('\n') + 1;
  if (pos <= 0) return null;
  while (pos < head.length) {
    let end = head.indexOf('\n', pos);
    if (end < 0) end = head.length;
    const line = head.slice(pos, end).replace(/\r$/, '');
    if (line === '') return null; // end of head
    const colon = line.indexOf(':');
    if (colon > 0 && line.slice(0, colon).trim().toLowerCase() === 'host') return pos + line.length;
    pos = end + 1;
  }
  return null;
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

// ---- tab <-> draft json ----

const FOLLOWS: readonly Follow[] = ['off', 'in-scope', 'always'];

/** Serialize a tab to the draft `json`. `raw` is a Latin-1 string (byte-exact). */
export function toJson(tab: RepeaterTab): unknown {
  const json: Record<string, unknown> = { conn: { ...tab.conn }, mode: tab.mode, raw: bytesToLatin1(tab.raw) };
  if (tab.title) json.title = tab.title;
  json.history = (tab.history ?? []).map((r) => ({ ...r, conn: { ...r.conn } }));
  return json;
}

const TITLE_LEN = 120;

function cleanTitle(v: unknown): string | undefined {
  if (typeof v !== 'string') return undefined;
  const t = v.trim();
  return t === '' ? undefined : t.slice(0, TITLE_LEN);
}

function connFrom(c: Partial<TabConn>): TabConn {
  const conn: TabConn = {
    host: typeof c.host === 'string' ? c.host : '',
    port: typeof c.port === 'number' ? c.port : 80,
    tls: c.tls === true,
    reuse: c.reuse === true,
    follow: FOLLOWS.includes(c.follow as Follow) ? (c.follow as Follow) : 'off',
  };
  if (typeof c.sni === 'string') conn.sni = c.sni;
  if (typeof c.verify === 'string') conn.verify = c.verify;
  return conn;
}

const isNum = (v: unknown): v is number => typeof v === 'number' && Number.isFinite(v);
const isStr = (v: unknown): v is string => typeof v === 'string';

/** One stored history record, field by field; null if it is not one we wrote. */
function recordFrom(v: unknown): SendRecord | null {
  if (typeof v !== 'object' || v === null) return null;
  const j = v as Record<string, unknown>;
  if (!isStr(j.at) || !isStr(j.request_sha256) || !isNum(j.request_len)) return null;
  const rec: SendRecord = {
    at: j.at,
    request_sha256: j.request_sha256,
    request_len: j.request_len,
    conn: connFrom(typeof j.conn === 'object' && j.conn !== null ? (j.conn as Partial<TabConn>) : {}),
    mode: j.mode === 'assisted' ? 'assisted' : 'raw',
  };
  if (isNum(j.status)) rec.status = j.status;
  if (isNum(j.ttfb_ms)) rec.ttfb_ms = j.ttfb_ms;
  if (isNum(j.total_ms)) rec.total_ms = j.total_ms;
  if (isNum(j.size)) rec.size = j.size;
  if (isStr(j.exchange_id)) rec.exchange_id = j.exchange_id;
  if (isStr(j.scope_state)) rec.scope_state = j.scope_state;
  if (isStr(j.error)) rec.error = j.error;
  if (isStr(j.request) && j.truncated !== true) rec.request = j.request;
  else rec.truncated = true;
  return rec;
}

/** Parse a draft `json` back into a tab; null if it is not a draft we wrote. */
export function fromJson(id: string, json: unknown): RepeaterTab | null {
  if (typeof json !== 'object' || json === null) return null;
  const j = json as { conn?: Partial<TabConn>; mode?: unknown; raw?: unknown; title?: unknown; history?: unknown };
  if (typeof j.raw !== 'string') return null;
  const history: SendRecord[] = [];
  if (Array.isArray(j.history)) {
    for (const v of j.history) {
      if (history.length >= HISTORY_MAX) break;
      const rec = recordFrom(v);
      if (rec) history.push(rec);
    }
  }
  const tab: RepeaterTab = {
    id,
    conn: connFrom(j.conn ?? {}),
    mode: j.mode === 'assisted' ? 'assisted' : 'raw',
    raw: latin1ToBytes(j.raw),
    history,
  };
  const title = cleanTitle(j.title);
  if (title) tab.title = title;
  return tab;
}

const utf8Len = (s: string) => new TextEncoder().encode(s).length;

/**
 * Keep the draft JSON within DRAFT_BUDGET: drop request bytes from the oldest
 * records first (marking them truncated, the digest stays), then, if the draft
 * is still too big, the oldest records themselves. Mutates `tab.history`.
 */
export function fitDraft(tab: RepeaterTab): void {
  const history = tab.history;
  if (!history || history.length === 0) return;
  const base = utf8Len(JSON.stringify(toJson({ ...tab, history: [] })));
  const sizes = history.map((r) => utf8Len(JSON.stringify(r)) + 1);
  let total = base + sizes.reduce((a, b) => a + b, 0);
  for (let i = history.length - 1; i >= 0 && total > DRAFT_BUDGET; i--) {
    const r = history[i];
    if (r.request === undefined) continue;
    delete r.request;
    r.truncated = true;
    const size = utf8Len(JSON.stringify(r)) + 1;
    total += size - sizes[i];
    sizes[i] = size;
  }
  while (history.length > 0 && total > DRAFT_BUDGET) {
    history.pop();
    total -= sizes.pop() ?? 0;
  }
}

interface HeadLike {
  response?: { head?: { type?: string; bytes?: string; map?: [string, string][] } } | null;
}

/** An exchange's response head as Latin-1 text ('' when there is none). */
function responseHeadText(ex: unknown): string {
  const h = (ex as HeadLike | null)?.response?.head;
  if (!h) return '';
  try {
    if (h.type === 'http1-raw' && typeof h.bytes === 'string') return atob(h.bytes);
    if (h.type === 'normalized') {
      const lines = (h.map ?? []).map(([name, v]) => `${name}: ${atob(v)}`);
      return lines.length > 0 ? `${lines.join('\r\n')}\r\n\r\n` : '';
    }
  } catch {
    return '';
  }
  return '';
}

/**
 * A response's Raw text for comparing: the head (raw bytes, or `name: value`
 * lines for a normalized head) followed by the body, all read as Latin-1.
 */
export function responseRawText(ex: unknown, body: Uint8Array): string {
  return responseHeadText(ex) + bytesToLatin1(body);
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
        fitDraft(tab);
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

  /**
   * Open a tab: seeded (send to repeater) or, without `raw`, the GET / template
   * with the caret put on its Host header (unless `focus` is false, as for the
   * tab the view opens by itself on mount).
   */
  function newTab(seed: TabSeed = {}, opts: { focus?: boolean } = {}): string {
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
      raw: seed.raw ?? latin1ToBytes(NEW_TAB_TEMPLATE),
      history: [],
    };
    if (!seed.raw && opts.focus !== false) tab.focusHost = true;
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

  function sameBytes(a: Uint8Array, b: Uint8Array): boolean {
    if (a === b) return true;
    if (a.length !== b.length) return false;
    for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false;
    return true;
  }

  function setRaw(id: string, raw: Uint8Array): void {
    const t = find(id);
    if (!t) return;
    // The editor reports its document once at mount: that is not an edit.
    if (sameBytes(t.raw, raw)) return;
    t.raw = raw;
    t.dirty = true;
    schedule(id);
  }

  /** Called once by the editor after it has put the caret on a fresh tab's Host header. */
  function hostFocused(id: string): void {
    const t = find(id);
    if (t) t.focusHost = false;
  }

  /** The tab-strip title: an explicit rename, else derived from the draft. */
  function titleFor(tab: RepeaterTab): string {
    if (tab.title) return tab.title;
    const i = tabs.findIndex((t) => t.id === tab.id);
    return deriveTitle(tab.raw, (i < 0 ? tabs.length : i) + 1);
  }

  /** Rename a tab (saved in its draft). A blank name reverts to the derived title. */
  function renameTab(id: string, title: string): void {
    const t = find(id);
    if (!t) return;
    t.title = cleanTitle(title);
    schedule(id);
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
    const sent = t.raw;
    const base: SendRecord = {
      at: isoNow(),
      request_sha256: sha256Hex(sent),
      request_len: sent.length,
      conn: { ...t.conn },
      mode: t.mode,
    };
    if (sent.length <= HISTORY_REQUEST_MAX) base.request = body.raw;
    else base.truncated = true;
    t.sending = true;
    t.error = undefined;
    let rec: SendRecord;
    try {
      const outcome = await api.sendRequest(body);
      rec = {
        ...base,
        exchange_id: outcome.exchange_id,
        scope_state: outcome.scope_state,
        ttfb_ms: outcome.ttfb_ms,
        total_ms: outcome.total_ms,
      };
      const cur = find(id);
      if (cur) cur.outcome = outcome;
    } catch (e) {
      // 403 out-of-scope / 422 unparseable / 502 upstream: show it, keep the draft.
      rec = { ...base, error: messageOf(e) };
      const cur = find(id);
      if (cur) cur.error = messageOf(e);
      toasts.push({ kind: 'error', text: `Send failed: ${messageOf(e)}` });
    } finally {
      const cur = find(id);
      if (cur) cur.sending = false;
    }
    const cur = find(id);
    if (!cur) return;
    const history = [rec, ...(cur.history ?? [])].slice(0, HISTORY_MAX);
    cur.history = history;
    cur.notice = undefined;
    // Edits made while the send was in flight stay unsent.
    if (cur.raw === sent) cur.dirty = false;
    fitDraft(cur);
    schedule(id);
  }

  /** Fill a record's status and size once its response has been read (the response pane calls this). */
  function noteResponse(id: string, exchangeId: string, r: { status?: number | null; size?: number | null }): void {
    const rec = find(id)?.history?.find((x) => x.exchange_id === exchangeId);
    if (!rec) return;
    let changed = false;
    if (typeof r.status === 'number' && rec.status !== r.status) {
      rec.status = r.status;
      changed = true;
    }
    if (typeof r.size === 'number' && rec.size !== r.size) {
      rec.size = r.size;
      changed = true;
    }
    if (changed) schedule(id);
  }

  /**
   * Put a recorded send back: its request bytes, target and mode go into the
   * editor (marked dirty; nothing is sent) and its response is shown again,
   * fetched by exchange id. A record without its bytes restores the response
   * only and says so. A failed send shows its error.
   */
  function restore(id: string, index: number): void {
    const t = find(id);
    const rec = t?.history?.[index];
    if (!t || !rec) return;
    const stamp = timeShort(rec.at) || 'an unknown time';
    if (rec.request !== undefined && !rec.truncated) {
      let bytes: Uint8Array;
      try {
        bytes = unb64(rec.request);
      } catch {
        t.notice = `The request of the send at ${stamp} could not be read.`;
        return;
      }
      t.raw = bytes;
      t.conn = { ...rec.conn };
      t.mode = rec.mode;
      t.dirty = true;
      t.rev = (t.rev ?? 0) + 1;
      t.notice = `Restored the request sent at ${stamp}. Not sent yet.`;
      schedule(id);
    } else {
      t.notice = `The request sent at ${stamp} was too large to keep (${rec.request_len} bytes, SHA-256 ${rec.request_sha256.slice(0, 12)}…). Showing its response only; the editor is unchanged.`;
    }
    if (rec.exchange_id) {
      t.outcome = {
        exchange_id: rec.exchange_id,
        scope_state: rec.scope_state ?? '',
        changes: [],
        skipped: [],
        cert_override: false,
        ttfb_ms: rec.ttfb_ms ?? 0,
        total_ms: rec.total_ms ?? 0,
        redirects: [],
      };
      t.error = undefined;
    } else {
      t.outcome = undefined;
      t.error = rec.error ?? 'send failed';
    }
  }

  function clearHistory(id: string): void {
    const t = find(id);
    if (!t) return;
    t.history = [];
    t.notice = undefined;
    schedule(id);
  }

  /**
   * The two responses "Compare with previous" diffs: the one shown now (or
   * the newest with a response) and the next older send that has a response.
   */
  function comparePair(tab: RepeaterTab): { newer: SendRecord; older: SendRecord } | null {
    const h = tab.history ?? [];
    const shown = tab.outcome ? h.findIndex((r) => r.exchange_id === tab.outcome?.exchange_id) : -1;
    const start = shown >= 0 ? shown : h.findIndex((r) => r.exchange_id);
    if (start < 0) return null;
    const prev = h.findIndex((r, i) => i > start && r.exchange_id);
    return prev < 0 ? null : { newer: h[start], older: h[prev] };
  }

  /** A response's Raw text by exchange id. A missing body (404) is empty. */
  async function responseText(exchangeId: string): Promise<string> {
    const [ex, body] = await Promise.all([
      api.getExchange(exchangeId),
      api.getRespBody(exchangeId).catch((e: unknown) => {
        if (e instanceof ApiError && e.status === 404) return new Uint8Array();
        throw e;
      }),
    ]);
    return responseRawText(ex, body);
  }

  /** Load both sides of comparePair as Raw text; null when there is nothing to compare. */
  async function compareWithPrevious(
    id: string,
  ): Promise<{ newer: SendRecord; older: SendRecord; newerText: string; olderText: string } | null> {
    const t = find(id);
    const pair = t ? comparePair(t) : null;
    if (!pair) return null;
    const [newerText, olderText] = await Promise.all([
      responseText(pair.newer.exchange_id!),
      responseText(pair.older.exchange_id!),
    ]);
    return { ...pair, newerText, olderText };
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
    restore,
    clearHistory,
    noteResponse,
    hostFocused,
    comparePair,
    compareWithPrevious,
    loadTabs,
    flushDrafts,
    sendFromExchange,
  };
}

export const repeater = createRepeater();
