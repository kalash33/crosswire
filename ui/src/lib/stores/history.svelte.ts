// The history store: the recorded-exchange list with filters, keyset paging
// and live updates. It wires the Task 3 API client and the Task 5 event
// stream into the Task 8 History view.
//
// alpha.2 exact shapes (the server handlers own these; openapi.json leaves the
// exchange rows/events untyped, so they are defined here from the DTOs):
//
//   GET /api/v1/exchanges  ->  PageDto<ExchangeRowDto>
//   query params = ExchangeFilter (+ limit/after)
//
// A ROW is an ExchangeRowDto plus a local `uncommitted` flag the live stream
// toggles (a `received` event marks it, a `committed` event clears it).

import { apiGet, type QueryParams } from '../api/client';
import { events, type Event } from './events.svelte';

/** One history row, exactly the fields alpha.2 provides (no mime/tls/ms). */
export interface ExchangeRowDto {
  id: string;
  /** Creation time, unix MILLIS. */
  created_at: number;
  method: string | null;
  host: string | null;
  path: string | null;
  status: number | null;
  /** Response body length. */
  resp_len: number | null;
  /** Recording state wire name (`received`/`committed`/`error`). */
  state: string;
  tags: string[];
}

/** A row as the store holds it: the DTO plus the live uncommitted flag. */
export interface Row extends ExchangeRowDto {
  /** True while the exchange has not been durably committed. */
  uncommitted: boolean;
}

/** One page of rows, keyed by an opaque keyset cursor. */
export interface PageDto<T> {
  items: T[];
  /** Pass as `after` for the next page; null at the end. */
  next_cursor: string | null;
}

/** The `GET /exchanges` filter fields (all optional). */
export interface ExchangeFilter {
  method?: string;
  host?: string;
  host_contains?: string;
  status_min?: number;
  status_max?: number;
  namespace?: string;
  state?: string;
  scope_state?: string;
  tag?: string;
}

export interface HistoryOptions {
  /** Injectable JSON GET (defaults to the Task 3 client). */
  get?: <T>(path: string, params?: QueryParams) => Promise<T>;
  /** Injectable event subscription (defaults to the Task 5 store). */
  subscribe?: (onEvent: (e: Event) => void, onLagged: () => void) => () => void;
  /** Page size / live-row cap tuning. */
  pageSize?: number;
  /** Upper bound on retained rows under a live-event flood. */
  maxRows?: number;
}

const EXCHANGES = '/api/v1/exchanges';

function toRow(dto: ExchangeRowDto): Row {
  return { ...dto, uncommitted: dto.state !== 'committed' };
}

/** A minimal placeholder row for a `received` event we have no row for yet. */
function placeholderRow(id: string): Row {
  return {
    id,
    created_at: Date.now(),
    method: null,
    host: null,
    path: null,
    status: null,
    resp_len: null,
    state: 'received',
    tags: [],
    uncommitted: true,
  };
}

export function createHistory(opts: HistoryOptions = {}) {
  const get = opts.get ?? apiGet;
  const subscribe = opts.subscribe ?? ((oe, ol) => events.subscribe(oe, ol));
  const pageSize = opts.pageSize ?? 100;
  const maxRows = opts.maxRows ?? 5_000;

  let rows = $state<Row[]>([]);
  let activeFilter = $state<ExchangeFilter>({});
  let loading = $state(false);
  let error = $state<string | null>(null);
  // Plain (non-reactive) paging cursor: null means "no more / not yet loaded".
  let nextCursor: string | null = null;
  let loaded = false;

  function params(after: string | undefined): QueryParams {
    const f = activeFilter;
    // undefined/null values are dropped by the client's query serializer.
    return {
      limit: pageSize,
      after,
      method: f.method,
      host: f.host,
      host_contains: f.host_contains,
      status_min: f.status_min,
      status_max: f.status_max,
      namespace: f.namespace,
      state: f.state,
      scope_state: f.scope_state,
      tag: f.tag,
    };
  }

  async function fetchPage(after: string | undefined, append: boolean): Promise<void> {
    loading = true;
    error = null;
    try {
      const page = await get<PageDto<ExchangeRowDto>>(EXCHANGES, params(after));
      const mapped = page.items.map(toRow);
      rows = append ? [...rows, ...mapped] : mapped;
      nextCursor = page.next_cursor;
      loaded = true;
    } catch (e) {
      error = e instanceof Error ? e.message : 'failed to load history';
    } finally {
      loading = false;
    }
  }

  /** Replace the filter and reload from the first page. */
  async function applyFilter(filter: ExchangeFilter): Promise<void> {
    activeFilter = { ...filter };
    nextCursor = null;
    loaded = false;
    await fetchPage(undefined, false);
  }

  /** Reload the current view from the first page (keeps the filter). */
  async function reload(): Promise<void> {
    nextCursor = null;
    await fetchPage(undefined, false);
  }

  /** Append the next keyset page, using the stored `next_cursor` as `after`. */
  async function loadMore(): Promise<void> {
    if (!loaded || nextCursor === null) return;
    await fetchPage(nextCursor, true);
  }

  function onEvent(ev: Event): void {
    if (ev.type === 'received') {
      const idx = rows.findIndex((r) => r.id === ev.id);
      if (idx >= 0) {
        const next = rows.slice();
        next[idx] = { ...next[idx], uncommitted: true };
        rows = next;
      } else {
        // A new exchange arrived: prepend a placeholder, bounded.
        rows = [placeholderRow(ev.id), ...rows].slice(0, maxRows);
      }
    } else if (ev.type === 'committed') {
      const ids = ev.ids;
      let changed = false;
      const next = rows.map((r) => {
        if (ids.includes(r.id) && (r.uncommitted || r.state !== 'committed')) {
          changed = true;
          return { ...r, uncommitted: false, state: 'committed' };
        }
        return r;
      });
      if (changed) rows = next;
    }
  }

  // On lagged/reconnect the delta we missed is unknown, so refetch the view
  // rather than assume continuity.
  const unsubscribe = subscribe(onEvent, () => void reload());

  return {
    get rows(): readonly Row[] {
      return rows;
    },
    get filter(): ExchangeFilter {
      return activeFilter;
    },
    get loading(): boolean {
      return loading;
    },
    get error(): string | null {
      return error;
    },
    get hasMore(): boolean {
      return nextCursor !== null;
    },
    applyFilter,
    reload,
    loadMore,
    dispose(): void {
      unsubscribe();
    },
  };
}

/** The shared history store for the app. */
export const history = createHistory();

// ---------------------------------------------------------------------------
// Detail-view types (a pragmatic subset of crosswire_model::Exchange, rendered
// by ExchangeDetail). Byte payloads (head bytes, normalized header values)
// arrive base64-encoded per the model's bytes_serde wire choice.
// ---------------------------------------------------------------------------

/** One header field's spans into the raw head bytes. */
export interface HeaderSpanDto {
  name: { start: number; end: number };
  value: { start: number; end: number };
  line: { start: number; end: number };
}

/** The head of a message (serde-tagged by `type`). */
export type HeadDto =
  | {
      type: 'http1-raw';
      /** Raw head bytes (base64), including the terminating empty line. */
      bytes: string;
      spans?: unknown;
      parser_version?: number;
    }
  | {
      type: 'normalized';
      /** `[name, valueBytesBase64][]` in order. */
      map?: [string, string][];
    }
  | {
      type: 'tunnel';
      bytes_in: number;
      bytes_out: number;
      duration_ms: number;
      reason: string;
    };

/** A parse finding on the message. */
export interface DiagnosticDto {
  code: string;
  span: { start: number; end: number };
  severity: string;
}

/** The body of a message (serde-tagged by `type`). */
export interface BodyDto {
  type: string;
  truncated_at?: number | null;
}

/** One recorded request or response. */
export interface MessageDto {
  kind: string;
  fidelity: string;
  head: HeadDto;
  diagnostics?: DiagnosticDto[];
  body: BodyDto;
}

/** The full exchange returned by `GET /exchanges/{id}`. */
export interface ExchangeDto {
  id: string;
  request: MessageDto;
  response?: MessageDto | null;
  scope_state?: string;
  state?: string;
  tags?: string[];
  notes?: string;
}
