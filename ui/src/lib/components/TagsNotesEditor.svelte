<script lang="ts">
  import { toasts } from '../ui/toasts.svelte';
  import { onDestroy, untrack } from 'svelte';
  import { patchExchange } from '../api/exchanges';
  import { addTag } from '../tags';

  type Saved = { id: string; tags?: string[]; notes?: string };

  // Tags and notes are user-controlled text: rendered ONLY via `{…}` text
  // interpolation (§11.1) — never as markup, never in an href/src.
  let {
    id,
    tags,
    notes,
    onSaved,
  }: { id: string; tags: string[]; notes: string; onSaved: (saved: Saved) => void } = $props();

  const NOTES_DEBOUNCE_MS = 400;

  // One lane per field per exchange. The PATCH routes REPLACE the whole field
  // and the server applies requests in arrival order, so at most ONE save per
  // lane is in flight; while it is, only the LATEST wanted value is remembered
  // (`next`) and sent when the in-flight save settles. A lane goes `stale` when
  // its exchange is left: it keeps draining (so a pending value is still
  // delivered to the right id) but no longer touches local UI state.
  interface Lane<T> {
    busy: boolean;
    next: { v: T } | null;
    stale: boolean;
  }
  const newLane = <T,>(): Lane<T> => ({ busy: false, next: null, stale: false });

  async function drain<T>(
    lane: Lane<T>,
    send: (v: T) => Promise<void>,
    onOk: (v: T) => void,
    onFail: (v: T, e: unknown) => void,
  ): Promise<void> {
    lane.busy = true;
    try {
      while (lane.next) {
        const { v } = lane.next;
        lane.next = null;
        try {
          await send(v);
        } catch (e) {
          // A newer value supersedes the failed one: it is sent next.
          if (!lane.stale && !lane.next) onFail(v, e);
          continue;
        }
        onOk(v);
      }
    } finally {
      lane.busy = false;
    }
  }

  let localTags = $state<string[]>(untrack(() => [...tags]));
  let localNotes = $state(untrack(() => notes));
  let draft = $state('');
  let tagError = $state<string | null>(null);
  let saveError = $state<string | null>(null);
  let timer: ReturnType<typeof setTimeout> | null = null;
  let notesDirty = false;
  // Exchange the dirty notes belong to, captured when the save is SCHEDULED.
  let notesId = untrack(() => id);
  // Last values the server is known to hold (rollback targets).
  let tagServer = untrack(() => [...tags]);
  let notesServer = untrack(() => notes);
  let tagLane = newLane<string[]>();
  let notesLane = newLane<string>();

  // A different exchange: flush dirty notes for the OLD id, then reset local
  // state. The parent reassigns `exchange` after every save, which re-fires this
  // effect with the SAME id; resetting then would wipe a draft or pending notes
  // typed while the save was in flight, so only a changed id resets. A same-id
  // prop refresh is deliberately NOT adopted: the parent never re-fetches the
  // same id.
  let lastId = untrack(() => id);
  $effect(() => {
    const next = id;
    untrack(() => {
      if (next === lastId) return;
      lastId = next;
      saveNotes(); // targets notesId (the old exchange)
      tagLane.stale = true;
      notesLane.stale = true;
      tagLane = newLane();
      notesLane = newLane();
      localTags = [...tags];
      localNotes = notes;
      tagServer = [...tags];
      notesServer = notes;
      notesId = next;
      draft = '';
      tagError = null;
      saveError = null;
      notesDirty = false;
    });
  });

  // Unmount: drop the debounce timer but still deliver dirty notes (and any
  // coalesced tag value) to the exchange they were typed for.
  onDestroy(() => {
    saveNotes();
    tagLane.stale = true;
    notesLane.stale = true;
  });

  function saveTags(next: string[]): void {
    const lane = tagLane;
    const savedId = id;
    localTags = next; // optimistic
    saveError = null;
    lane.next = { v: next };
    if (lane.busy) return;
    void drain(
      lane,
      (v) => patchExchange(savedId, { tags: v }),
      (v) => {
        if (!lane.stale) tagServer = v;
        onSaved({ id: savedId, tags: v });
      },
      (_v, e) => {
        localTags = tagServer;
        saveError = e instanceof Error ? e.message : 'failed to save tags';
        toasts.push({ kind: 'error', text: `Tags not saved: ${saveError}` });
      },
    );
  }

  function submitTag(): void {
    const res = addTag(localTags, draft);
    if (!res.ok) {
      tagError = res.error;
      return;
    }
    tagError = null;
    draft = '';
    void saveTags(res.tags);
  }

  function removeTag(tag: string): void {
    tagError = null;
    void saveTags(localTags.filter((t) => t !== tag));
  }

  function saveNotes(): void {
    if (timer) clearTimeout(timer);
    timer = null;
    if (!notesDirty) return;
    notesDirty = false;
    const lane = notesLane;
    const savedId = notesId;
    const sent = localNotes;
    saveError = null;
    lane.next = { v: sent };
    if (lane.busy) return;
    void drain(
      lane,
      (v) => patchExchange(savedId, { notes: v }),
      (v) => {
        if (!lane.stale) notesServer = v;
        onSaved({ id: savedId, notes: v });
      },
      (v, e) => {
        if (localNotes === v) localNotes = notesServer;
        else notesDirty = true; // keep text typed since; it will be re-sent
        saveError = e instanceof Error ? e.message : 'failed to save notes';
        toasts.push({ kind: 'error', text: `Notes not saved: ${saveError}` });
      },
    );
  }

  function onNotesInput(e: Event): void {
    localNotes = (e.currentTarget as HTMLTextAreaElement).value;
    notesDirty = true;
    notesId = id; // the exchange these edits belong to
    if (timer) clearTimeout(timer);
    timer = setTimeout(saveNotes, NOTES_DEBOUNCE_MS);
  }
</script>

<div class="tn">
  <div class="tags">
    {#each localTags as tag, i (i)}
      <span class="chip tag" data-testid="tag-chip"
        >{tag}<button
          type="button"
          class="rm"
          data-testid="tag-remove"
          aria-label={`remove tag ${tag}`}
          onclick={() => removeTag(tag)}>×</button
        ></span
      >
    {/each}
    <input
      class="tag-input"
      data-testid="tag-input"
      placeholder="add tag"
      aria-label="add tag"
      bind:value={draft}
      onkeydown={(e) => {
        if (e.key === 'Enter') {
          e.preventDefault();
          submitTag();
        }
      }}
    />
  </div>
  {#if tagError}<p class="error" role="alert" data-testid="tag-error">{tagError}</p>{/if}
  <textarea
    class="notes"
    data-testid="notes-input"
    placeholder="notes"
    aria-label="notes"
    rows="2"
    value={localNotes}
    oninput={onNotesInput}
    onblur={saveNotes}
  ></textarea>
  {#if saveError}<p class="error" role="alert" data-testid="tags-notes-error">{saveError}</p>{/if}
</div>

<style>
  .tn {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
    flex-basis: 100%;
  }
  .tags {
    display: flex;
    flex-wrap: wrap;
    gap: 0.3rem;
    align-items: center;
  }
  .chip.tag {
    font-size: 0.75rem;
    padding: 0.1rem 0.4rem;
    border-radius: 3px;
    background: var(--chip-bg);
    color: var(--chip-fg);
  }
  .rm {
    margin-left: 0.25rem;
    padding: 0 0.2rem;
    background: none;
    border: none;
    color: inherit;
    cursor: pointer;
  }
  .tag-input {
    padding: 0.15rem 0.35rem;
    font-size: 0.75rem;
    width: 8rem;
  }
  .notes {
    width: 100%;
    font-size: 0.8rem;
    resize: vertical;
    box-sizing: border-box;
  }
  .error {
    margin: 0;
    font-size: 0.75rem;
  }
</style>
