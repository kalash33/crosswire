// Transient notifications (U1 spec D2). Text is rendered by Toast.svelte as
// plain text interpolation, so captured values are safe to include.

export type ToastKind = 'info' | 'success' | 'error';

export interface ToastItem {
  id: number;
  kind: ToastKind;
  text: string;
}

export interface ToastInput {
  kind?: ToastKind;
  text: string;
  /** Auto-dismiss delay; default 4000 ms. */
  ttlMs?: number;
}

const DEFAULT_TTL_MS = 4000;
/** Older toasts are dropped beyond this many (no unbounded growth). */
const MAX_VISIBLE = 5;

function createToasts() {
  let items = $state<ToastItem[]>([]);
  // Timer handles are bookkeeping, not UI state: a plain (non-reactive) record.
  let timers: Record<number, ReturnType<typeof setTimeout>> = {};
  let nextId = 1;

  function arm(id: number, ttlMs: number) {
    const old = timers[id];
    if (old !== undefined) clearTimeout(old);
    timers[id] = setTimeout(() => dismiss(id), ttlMs);
  }

  function dismiss(id: number): void {
    const t = timers[id];
    if (t !== undefined) clearTimeout(t);
    delete timers[id];
    items = items.filter((i) => i.id !== id);
  }

  return {
    get items(): readonly ToastItem[] {
      return items;
    },
    /**
     * Show a toast; returns its id. The same kind+text as the newest toast
     * re-arms that toast instead of stacking a duplicate (no toast storms).
     */
    push({ kind = 'info', text, ttlMs = DEFAULT_TTL_MS }: ToastInput): number {
      const last = items.at(-1);
      if (last && last.text === text && last.kind === kind) {
        arm(last.id, ttlMs);
        return last.id;
      }
      const id = nextId++;
      items = [...items, { id, kind, text }];
      arm(id, ttlMs);
      while (items.length > MAX_VISIBLE) dismiss(items[0].id);
      return id;
    },
    dismiss,
    clear(): void {
      for (const t of Object.values(timers)) clearTimeout(t);
      timers = {};
      items = [];
    },
  };
}

export const toasts = createToasts();
