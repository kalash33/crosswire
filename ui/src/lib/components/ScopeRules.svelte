<script lang="ts">
  import { scope } from '../stores/scope.svelte';
  import { HOST_TYPES, PATH_TYPES } from '../api/scope';
  import {
    blankRow,
    draftFromScope,
    scopeFromDraft,
    RowError,
    type Draft,
  } from './scope-rows';

  // The draft is local: a failed save (422) never touches it, so the editor
  // stays populated with the user's edits. It is re-seeded only when the
  // store's scope object is replaced (load / successful save / applied import).
  let draft = $state<Draft>({ include: [], exclude: [] });
  let seededFrom: unknown = null;
  let localError = $state<string | null>(null);

  $effect(() => {
    const s = scope.scope;
    if (s === seededFrom) return;
    seededFrom = s;
    draft = draftFromScope(s);
    localError = null;
  });

  type List = 'include' | 'exclude';

  function add(list: List) {
    draft[list].push(blankRow());
  }
  function remove(list: List, i: number) {
    draft[list].splice(i, 1);
  }
  function move(list: List, i: number, d: -1 | 1) {
    const j = i + d;
    if (j < 0 || j >= draft[list].length) return;
    const rows = draft[list];
    [rows[i], rows[j]] = [rows[j], rows[i]];
  }

  async function save() {
    localError = null;
    let next;
    try {
      next = scopeFromDraft(draft);
    } catch (e) {
      if (e instanceof RowError) {
        localError = e.message;
        return;
      }
      throw e;
    }
    await scope.save(next);
  }

  const lists: { key: List; title: string }[] = [
    { key: 'include', title: 'Include' },
    { key: 'exclude', title: 'Exclude (wins over include)' },
  ];
</script>

<div class="rules" data-testid="scope-rules">
  {#each lists as l (l.key)}
    <section data-testid={`scope-${l.key}`}>
      <h3>{l.title}</h3>
      {#each draft[l.key] as row, i (row)}
        <div class="rule" data-testid="rule-row" data-list={l.key}>
          <select data-testid="rule-host-type" aria-label="host pattern type" bind:value={row.hostType}>
            {#each HOST_TYPES as t (t)}<option value={t}>{t}</option>{/each}
          </select>
          <input
            data-testid="rule-host-value"
            aria-label={row.hostType === 'suffix' ? 'suffix domain' : 'host value'}
            placeholder={row.hostType === 'suffix' ? 'example.com' : row.hostType}
            bind:value={row.hostValue}
          />
          {#if row.hostType === 'suffix'}
            <label
              ><input type="checkbox" data-testid="rule-also-apex" bind:checked={row.alsoApex} /> apex</label
            >
          {/if}
          <input
            data-testid="rule-ports"
            class="ports"
            aria-label="ports"
            placeholder="ports (any)"
            bind:value={row.portsText}
          />
          <select data-testid="rule-scheme" aria-label="scheme" bind:value={row.scheme}>
            <option value="">any scheme</option>
            <option value="http">http</option>
            <option value="https">https</option>
          </select>
          <select data-testid="rule-path-type" aria-label="path match type" bind:value={row.pathType}>
            <option value="">any path</option>
            {#each PATH_TYPES as t (t)}<option value={t}>path {t}</option>{/each}
          </select>
          {#if row.pathType}
            <input
              data-testid="rule-path-value"
              aria-label="path value"
              placeholder="/"
              bind:value={row.pathValue}
            />
          {/if}
          <button type="button" data-testid="rule-up" aria-label="move up" disabled={i === 0} onclick={() => move(l.key, i, -1)}>↑</button>
          <button type="button" data-testid="rule-down" aria-label="move down" disabled={i === draft[l.key].length - 1} onclick={() => move(l.key, i, 1)}>↓</button>
          <button type="button" data-testid="rule-remove" aria-label="remove rule" onclick={() => remove(l.key, i)}>✕</button>
        </div>
      {:else}
        <p class="empty">No {l.key} rules.</p>
      {/each}
      <button type="button" data-testid={`add-${l.key}`} onclick={() => add(l.key)}>Add {l.key} rule</button>
    </section>
  {/each}

  {#if localError}<p class="error" role="alert" data-testid="scope-local-error">{localError}</p>{/if}
  {#if scope.error}<p class="error" role="alert" data-testid="scope-error">{scope.error}</p>{/if}
  <div><button type="button" data-testid="scope-save" disabled={!scope.loaded} onclick={() => void save()}>Save scope</button></div>
</div>

<style>
  .rules {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }
  h3 {
    margin: 0 0 0.3rem;
    font-size: 0.9rem;
  }
  .rule {
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem;
    align-items: center;
    margin-bottom: 0.3rem;
  }
  .ports {
    width: 8rem;
  }
  .empty {
    opacity: 0.6;
    margin: 0 0 0.3rem;
  }
  .error {
    color: #e06060;
    margin: 0;
  }
</style>
