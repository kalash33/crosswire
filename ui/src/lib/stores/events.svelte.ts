// Live event stream: ONE same-origin WebSocket to /api/v1/events. The
// HttpOnly session cookie authenticates the upgrade, so no token is sent.
// Frame vocabulary mirrors crosswire_model::Event (serde tag = "type",
// kebab-case) plus the API-level `intercept` frame (no `seq`).

export type Event =
  | { type: 'received'; id: string; seq: number }
  | { type: 'committed'; ids: string[]; seq: number }
  | { type: 'marker'; marker: Record<string, unknown>; seq: number }
  | { type: 'error'; detail: string; seq: number }
  | { type: 'limit-exceeded'; detail: string; seq: number }
  | { type: 'lagged'; missed: number; seq: number }
  | { type: 'intercept'; held: number };

const KNOWN = new Set([
  'received',
  'committed',
  'marker',
  'error',
  'limit-exceeded',
  'lagged',
  'intercept',
]);

export interface EventStoreOptions {
  socketFactory?: (url: string) => WebSocket;
  maxEvents?: number;
  baseDelayMs?: number;
  maxDelayMs?: number;
}

type Sub = { onEvent: (e: Event) => void; onLagged: () => void };

function eventsUrl(): string {
  const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
  return `${proto}//${location.host}/api/v1/events`;
}

export function createEventStore(opts: EventStoreOptions = {}) {
  const factory = opts.socketFactory ?? ((url: string) => new WebSocket(url));
  const maxEvents = opts.maxEvents ?? 200;
  const baseDelay = opts.baseDelayMs ?? 500;
  const maxDelay = opts.maxDelayMs ?? 15_000;

  let latest = $state<Event[]>([]);
  let connected = $state(false);
  let subs: Sub[] = [];
  let socket: WebSocket | null = null;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let delay = baseDelay;
  let running = false;
  let attempts = 0;

  function announceLagged() {
    for (const s of subs) s.onLagged();
  }

  function handle(data: unknown) {
    if (typeof data !== 'string') return;
    let frame: unknown;
    try {
      frame = JSON.parse(data);
    } catch {
      return;
    }
    const type = (frame as { type?: unknown } | null)?.type;
    if (typeof type !== 'string' || !KNOWN.has(type)) return;
    const ev = frame as Event;
    if (ev.type === 'lagged') {
      announceLagged();
      return;
    }
    latest = [...latest, ev].slice(-maxEvents);
    for (const s of subs) s.onEvent(ev);
  }

  function scheduleReconnect() {
    if (!running || timer !== null) return;
    timer = setTimeout(() => {
      timer = null;
      connect();
    }, delay);
    delay = Math.min(delay * 2, maxDelay);
  }

  function connect() {
    if (!running) return;
    attempts += 1;
    const sock = factory(eventsUrl());
    socket = sock;
    sock.onopen = () => {
      if (socket !== sock) return;
      connected = true;
      delay = baseDelay;
      // Anything may have happened while we were away: make consumers refetch.
      // Only a clean first attempt skips this: after any failed attempt the
      // consumer may already have fetched its view before the socket was up.
      if (attempts > 1) announceLagged();
    };
    sock.onmessage = (e) => {
      if (socket === sock) handle(e.data);
    };
    sock.onclose = () => {
      if (socket !== sock) return;
      socket = null;
      connected = false;
      scheduleReconnect();
    };
    // An error is always followed by close; close drives the reconnect.
    sock.onerror = () => {};
  }

  return {
    get events(): readonly Event[] {
      return latest;
    },
    get connected(): boolean {
      return connected;
    },
    start() {
      if (running) return;
      running = true;
      connect();
    },
    close() {
      running = false;
      if (timer !== null) clearTimeout(timer);
      timer = null;
      const sock = socket;
      socket = null;
      connected = false;
      attempts = 0;
      delay = baseDelay;
      sock?.close();
    },
    subscribe(onEvent: (e: Event) => void, onLagged: () => void): () => void {
      const sub = { onEvent, onLagged };
      subs = [...subs, sub];
      return () => {
        subs = subs.filter((x) => x !== sub);
      };
    },
  };
}

export const events = createEventStore();
