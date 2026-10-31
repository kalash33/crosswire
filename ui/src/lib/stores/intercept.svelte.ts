// Request intercept store. Wire notes (crates/crosswire-api/src/routes/intercept.rs):
//  - ForwardBody.head/body are STANDARD base64. An edited body REQUIRES an
//    edited head (server 422), so an edit always carries the head.
//  - Omitting both forwards the ORIGINAL bytes — the path for non-editable
//    (over-limit) items, never a truncation.
//  - Held head/body bytes come from GET /intercept/held/{id}/{head|body};
//    body 404 = no buffered body.
import * as wire from '../api/intercept';
import type { HeldDto, RulesDto, ForwardBody, InterceptStatus } from '../api/intercept';
import { bytesToLatin1 } from '../editor/encoding';
import { events, type Event } from './events.svelte';
import { toasts } from '../ui/toasts.svelte';

export type InterceptApi = typeof wire;

export interface EditedRequest {
  head: Uint8Array;
  body?: Uint8Array | null;
}

export interface InterceptOptions {
  api?: InterceptApi;
  subscribe?: (onEvent: (e: Event) => void, onLagged: () => void) => () => void;
}

const DEFAULT_RULES: RulesDto = {
  skip_extensions: [],
  skip_out_of_scope: false,
  overflow: 'forward-newest',
  on_timeout: 'forward',
};

const b64 = (b: Uint8Array) => btoa(bytesToLatin1(b));

/** Split a raw request at the first CRLFCRLF (head keeps the terminator). */
export function splitRequest(bytes: Uint8Array): { head: Uint8Array; body: Uint8Array } {
  for (let i = 0; i + 3 < bytes.length; i++) {
    if (bytes[i] === 13 && bytes[i + 1] === 10 && bytes[i + 2] === 13 && bytes[i + 3] === 10) {
      return { head: bytes.slice(0, i + 4), body: bytes.slice(i + 4) };
    }
  }
  return { head: bytes.slice(), body: new Uint8Array() };
}

export function concatBytes(a: Uint8Array, b: Uint8Array | null): Uint8Array {
  const out = new Uint8Array(a.length + (b?.length ?? 0));
  out.set(a, 0);
  if (b) out.set(b, a.length);
  return out;
}

export function createIntercept(opts: InterceptOptions = {}) {
  const api = opts.api ?? wire;
  const subscribe = opts.subscribe ?? ((a, b) => events.subscribe(a, b));

  let enabled = $state(false);
  let held = $state<HeldDto[]>([]);
  let rules = $state<RulesDto>({ ...DEFAULT_RULES });
  let error = $state<string | null>(null);
  let loaded = $state(false);
  let unsub: (() => void) | null = null;

  const msg = (e: unknown) => (e instanceof Error ? e.message : String(e));

  async function refresh(): Promise<void> {
    try {
      const s: InterceptStatus = await api.getStatus();
      enabled = s.enabled;
      rules = s.rules;
      held = s.held;
      loaded = true;
      error = null;
    } catch (e) {
      error = msg(e);
    }
  }

  /** Run an action; with `outcome`, toast how it went. */
  async function act(
    fn: () => Promise<void>,
    outcome?: { ok: string; fail: string },
  ): Promise<void> {
    try {
      await fn();
      error = null;
      if (outcome) toasts.push({ kind: 'success', text: outcome.ok });
    } catch (e) {
      error = msg(e);
      if (outcome) toasts.push({ kind: 'error', text: `${outcome.fail}: ${error}` });
    }
  }

  return {
    get enabled() {
      return enabled;
    },
    get held() {
      return held;
    },
    get rules() {
      return rules;
    },
    get error() {
      return error;
    },
    get loaded() {
      return loaded;
    },
    refresh,
    /** Idempotent: load once and keep in sync with the event stream. */
    start() {
      if (unsub) return;
      unsub = subscribe((e) => {
        if (e.type === 'intercept') void refresh();
      }, () => void refresh());
      void refresh();
    },
    stop() {
      unsub?.();
      unsub = null;
    },
    async setEnabled(on: boolean) {
      await act(async () => {
        await api.putEnabled(on);
        enabled = on;
      });
    },
    async forward(id: string, edited?: EditedRequest) {
      let body: ForwardBody | undefined;
      if (edited) {
        body = { head: b64(edited.head) };
        if (edited.body) body.body = b64(edited.body);
      }
      await act(async () => {
        await api.postForward(id, body);
        await refresh();
      }, edited
        ? { ok: 'Forwarded the edited request.', fail: 'Forward failed' }
        : { ok: 'Forwarded the request.', fail: 'Forward failed' });
    },
    async drop(id: string) {
      await act(async () => {
        await api.postDrop(id);
        await refresh();
      }, { ok: 'Dropped the request.', fail: 'Drop failed' });
    },
    async forwardAll() {
      await act(async () => {
        await api.postForwardAll();
        await refresh();
      }, { ok: 'Forwarded all held requests.', fail: 'Forward all failed' });
    },
    async putRules(next: RulesDto) {
      await act(async () => {
        await api.putRules(next);
        rules = next;
      });
    },
    /** Original head and (when buffered) body bytes of a held request. */
    async heldBytes(id: string): Promise<{ head: Uint8Array; body: Uint8Array | null }> {
      const [head, body] = await Promise.all([api.getHeadBytes(id), api.getBodyBytes(id)]);
      return { head, body };
    },
  };
}

export const intercept = createIntercept();
