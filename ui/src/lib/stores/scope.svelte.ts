// Scope store. The server validates a PUT before swapping, so a 422 leaves the
// live scope untouched; this store mirrors that: `scope` only changes on a
// successful load/save, never on a failed one.
// There is no GET for enforce_scope_strictly in the API, so `enforceStrict`
// is tracked locally (default false) and `strictKnown` stays false until this
// UI session has set it.
import * as wire from '../api/scope';
import type { Scope } from '../api/scope';

export type ScopeApi = Pick<
  typeof wire,
  'getScope' | 'putScope' | 'importScope' | 'setEnforceStrict'
>;

export interface ScopeOptions {
  api?: ScopeApi;
}

const clone = <T>(v: T): T => JSON.parse(JSON.stringify(v)) as T;
const msg = (e: unknown) => (e instanceof Error ? e.message : String(e));

export function createScope(opts: ScopeOptions = {}) {
  const api = opts.api ?? wire;

  let scope = $state<Scope>({ include: [], exclude: [] });
  let loaded = $state(false);
  let preview = $state<Scope | null>(null);
  let error = $state<string | null>(null);
  let enforceStrict = $state(false);
  let strictKnown = $state(false);

  async function load(): Promise<void> {
    try {
      const next = await api.getScope();
      // Keep the object identity when nothing changed, so an editor seeded
      // from `scope` is not re-seeded (and its unsaved edits clobbered).
      if (JSON.stringify(next) !== JSON.stringify(scope)) scope = next;
      loaded = true;
      error = null;
    } catch (e) {
      error = msg(e);
    }
  }

  async function save(next: Scope): Promise<boolean> {
    try {
      await api.putScope(next);
      scope = clone(next);
      error = null;
      return true;
    } catch (e) {
      error = msg(e); // prior scope deliberately untouched
      return false;
    }
  }

  return {
    get scope() {
      return scope;
    },
    get loaded() {
      return loaded;
    },
    get preview() {
      return preview;
    },
    get error() {
      return error;
    },
    get enforceStrict() {
      return enforceStrict;
    },
    get strictKnown() {
      return strictKnown;
    },
    load,
    save,
    clearError() {
      error = null;
    },
    async importList(text: string): Promise<boolean> {
      try {
        preview = await api.importScope(text);
        error = null;
        return true;
      } catch (e) {
        preview = null;
        error = msg(e);
        return false;
      }
    },
    discardPreview() {
      preview = null;
    },
    async applyPreview(): Promise<boolean> {
      if (!preview) return false;
      const ok = await save(preview);
      if (ok) preview = null;
      return ok;
    },
    async setEnforceStrict(on: boolean): Promise<void> {
      try {
        await api.setEnforceStrict(on);
        enforceStrict = on;
        strictKnown = true;
        error = null;
      } catch (e) {
        error = msg(e);
      }
    },
  };
}

export const scope = createScope();
