<script lang="ts">
  import { untrack } from 'svelte';
  import { patchExchange } from '../api/exchanges';
  import { addTag } from '../tags';

  // Tags and notes are user-controlled text: rendered ONLY via `{…}` text
  // interpolation (§11.1) — never as markup, never in an href/src.
  let {
    id,
    tags,
    notes,
    onSaved,
  }: { id: string; tags: string[]; notes: string; onSaved: (saved: { tags?: string[]; notes?: string }) => void } = $props();

  const NOTES_DEBOUNCE_MS = 400;

  let localTags = $state<string[]>(untrack(() => [...tags]));
  let localNotes = $state(untrack(() => notes));
  let draft = $state('');
  let tagError = $state<string | null>(null);
  let saveError = $state<string | null>(null);
  let timer: ReturnType<typeof setTimeout> | null = null;
  let notesDirty = false;
  // Per-field save sequence: only the latest save of a field may apply.
  let tagSeq = 0;
  let notesSeq = 0;

  // A different exchange: reset local state (and drop any pending notes save).
  // The parent reassigns `exchange` after every save, which re-fires this effect
  // with the SAME id; resetting then would wipe a draft or pending notes typed
  // while the save was in flight, so only a changed id resets.
  let lastId = untrack(() => id);
  $effect(() => {
    const next = id;
    untrack(() => {
      if (next === lastId) return;
      lastId = next;
      localTags = [...tags];
      localNotes = notes;
      draft = '';
      tagError = null;
      saveError = null;
      notesDirty = false;
      tagSeq++;
      notesSeq++;
      if (timer) clearTimeout(timer);
      timer = null;
    });
  });

  async function saveTags(next: string[]): Promise<void> {
    const current = id;
    const seq = ++tagSeq;
    const previous = localTags;
    localTags = next; // optimistic
    saveError = null;
    try {
      await patchExchange(current, { tags: next });
    } catch (e) {
      if (id !== current || seq !== tagSeq) return;
      localTags = previous;
      saveError = e instanceof Error ? e.message : 'failed to save tags';
      return;
    }
    if (id !== current || seq !== tagSeq) return;
    onSaved({ tags: next });
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

  async function saveNotes(): Promise<void> {
    if (timer) clearTimeout(timer);
    timer = null;
    if (!notesDirty) return;
    notesDirty = false;
    const current = id;
    const seq = ++notesSeq;
    const sent = localNotes;
    const previous = untrack(() => notes);
    saveError = null;
    try {
      await patchExchange(current, { notes: sent });
    } catch (e) {
      if (id !== current || seq !== notesSeq) return;
      if (localNotes === sent) localNotes = previous;
      else notesDirty = true; // keep text typed since; it will be re-sent
      saveError = e instanceof Error ? e.message : 'failed to save notes';
      return;
    }
    if (id !== current || seq !== notesSeq) return;
    onSaved({ notes: sent });
  }

  function onNotesInput(e: Event): void {
    localNotes = (e.currentTarget as HTMLTextAreaElement).value;
    notesDirty = true;
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => void saveNotes(), NOTES_DEBOUNCE_MS);
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
    onblur={() => void saveNotes()}
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
    background: #223322;
    color: #bfe9bf;
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
