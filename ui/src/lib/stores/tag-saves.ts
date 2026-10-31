// In-flight tag saves from the detail's TagsNotesEditor lane, per exchange id.
// A bulk tag run waits for these to settle before reading that exchange's tags,
// so its REPLACE PATCH can never land on top of (and drop) a pending edit.

const inflight = new Map<string, Promise<void>>();

/** Record a lane drain for `id`; it is forgotten once it settles. */
export function trackTagSave(id: string, p: Promise<void>): void {
  inflight.set(id, p);
  const clear = () => {
    if (inflight.get(id) === p) inflight.delete(id);
  };
  p.then(clear, clear);
}

/** True while a tag save for `id` is in flight. */
export function tagSavePending(id: string): boolean {
  return inflight.has(id);
}

/** Resolves once no tag save for `id` is in flight (never rejects). */
export async function tagSavesSettled(id: string): Promise<void> {
  let p = inflight.get(id);
  while (p) {
    try {
      await p;
    } catch {
      // The lane reports its own failure; only completion matters here.
    }
    const next = inflight.get(id);
    p = next === p ? undefined : next;
  }
}
