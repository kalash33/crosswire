// Plain id -> index maps for the history store. Kept out of the `.svelte.ts`
// store because these are non-reactive lookup tables (rebuilt per array), not
// reactive state.

export function buildIndex(items: readonly { id: string }[]): Map<string, number> {
  const m = new Map<string, number>();
  for (let i = 0; i < items.length; i++) m.set(items[i].id, i);
  return m;
}

export function newIndex(): Map<string, number> {
  return new Map<string, number>();
}
