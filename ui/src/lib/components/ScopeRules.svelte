<script lang="ts">
  import { scope } from '../stores/scope.svelte';
  import { HOST_TYPES, PATH_TYPES } from '../api/scope';
  import Button from '../ui/Button.svelte';
  import EmptyState from '../ui/EmptyState.svelte';
  import IconButton from '../ui/IconButton.svelte';
  import Input from '../ui/Input.svelte';
  import Select from '../ui/Select.svelte';
  import { toasts } from '../ui/toasts.svelte';
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
    const ok = await scope.save(next);
    if (ok) {
      draft = draftFromScope(scope.scope); // canonical text, so the bar reads clean
      toasts.push({ kind: 'success', text: 'Scope saved' });
    }
    else toasts.push({ kind: 'error', text: scope.error ?? 'Could not save scope' });
  }

  const dirty = $derived(JSON.stringify(draft) !== JSON.stringify(draftFromScope(scope.scope)));

  const lists: { key: List; title: string }[] = [
    { key: 'include', title: 'Include' },
    { key: 'exclude', title: 'Exclude (wins over include)' },
  ];
</script>

{#each lists as l (l.key)}
  <section class="card" data-testid={`scope-${l.key}`}>
    <h2>{l.title}</h2>
    {#each draft[l.key] as row, i (row)}
      <div class="rule" data-testid="rule-row" data-list={l.key}>
        <Select data-testid="rule-host-type" aria-label="host pattern type" bind:value={row.hostType}>
          {#each HOST_TYPES as t (t)}<option value={t}>{t}</option>{/each}
        </Select>
        <Input
          data-testid="rule-host-value"
          class="host-value"
          aria-label={row.hostType === 'suffix' ? 'suffix domain' : 'host value'}
          placeholder={row.hostType === 'suffix' ? 'example.com' : row.hostType}
          bind:value={row.hostValue}
        />
        {#if row.hostType === 'suffix'}
          <label class="apex"
            ><input type="checkbox" data-testid="rule-also-apex" bind:checked={row.alsoApex} /> apex</label
          >
        {/if}
        <Input
          data-testid="rule-ports"
          class="ports"
          aria-label="ports"
          placeholder="ports (any)"
          bind:value={row.portsText}
        />
        <Select data-testid="rule-scheme" aria-label="scheme" bind:value={row.scheme}>
          <option value="">any scheme</option>
          <option value="http">http</option>
          <option value="https">https</option>
        </Select>
        <Select data-testid="rule-path-type" aria-label="path match type" bind:value={row.pathType}>
          <option value="">any path</option>
          {#each PATH_TYPES as t (t)}<option value={t}>path {t}</option>{/each}
        </Select>
        {#if row.pathType}
          <Input
            data-testid="rule-path-value"
            aria-label="path value"
            placeholder="/"
            bind:value={row.pathValue}
          />
        {/if}
        <span class="row-actions">
          <IconButton
            icon="chevron-down"
            label="Move rule up"
            class="flip"
            data-testid="rule-up"
            disabled={i === 0}
            onclick={() => move(l.key, i, -1)}
          />
          <IconButton
            icon="chevron-down"
            label="Move rule down"
            data-testid="rule-down"
            disabled={i === draft[l.key].length - 1}
            onclick={() => move(l.key, i, 1)}
          />
          <IconButton icon="close" label="Remove rule" data-testid="rule-remove" onclick={() => remove(l.key, i)} />
        </span>
      </div>
    {:else}
      {#if l.key === 'include'}
        <EmptyState
          icon="scope"
          title="No include rules"
          body="Everything is out of scope until you add an include rule."
        />
      {:else}
        <p class="empty">No exclude rules. Excludes win over includes.</p>
      {/if}
    {/each}
    <Button size="sm" icon="plus" data-testid={`add-${l.key}`} onclick={() => add(l.key)}>Add {l.key} rule</Button>
  </section>
{/each}

{#if localError}<p class="alert" role="alert" data-testid="scope-local-error">{localError}</p>{/if}
{#if scope.error}<p class="alert" role="alert" data-testid="scope-error">{scope.error}</p>{/if}

<div class="savebar" data-testid="scope-rules">
  <span class="dirty" class:on={dirty}>{dirty ? 'Unsaved changes' : 'All changes saved'}</span>
  <Button variant="primary" data-testid="scope-save" disabled={!scope.loaded} onclick={() => void save()}>Save scope</Button>
</div>

<style>
  .card {
    padding: var(--s-3) var(--s-4);
    background: var(--bg-elev);
    border: 1px solid var(--border);
    border-radius: var(--r-2);
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--s-2);
  }
  h2 {
    margin: 0;
    font-size: var(--fs-md);
    font-weight: 600;
  }
  .rule {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-2);
    align-items: center;
    align-self: stretch;
  }
  .rule :global(.host-value) {
    flex: 1 1 12rem;
  }
  .rule :global(.ports) {
    width: 8rem;
  }
  .rule :global(.flip svg) {
    transform: rotate(180deg);
  }
  .apex {
    display: inline-flex;
    align-items: center;
    gap: var(--s-1);
    color: var(--fg-muted);
  }
  .row-actions {
    display: inline-flex;
    gap: var(--s-1);
  }
  .empty {
    margin: 0;
    color: var(--fg-muted);
  }
  .alert {
    margin: 0;
    padding: var(--s-2) var(--s-3);
    color: var(--danger-fg);
    background: var(--danger-bg);
    border-radius: var(--r-1);
  }
  .savebar {
    order: 10;
    position: sticky;
    bottom: 0;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--s-3);
    padding: var(--s-2) var(--s-4);
    background: var(--bg-overlay);
    border: 1px solid var(--border-strong);
    border-radius: var(--r-2);
    box-shadow: var(--shadow-pop);
    z-index: 1;
  }
  .dirty {
    color: var(--fg-muted);
  }
  .dirty.on {
    color: var(--warn-fg);
    font-weight: 600;
  }
</style>
