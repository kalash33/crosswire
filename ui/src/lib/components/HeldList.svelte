<script lang="ts">
  import type { HeldDto } from '../api/intercept';

  // Held rows: host/method/path are attacker-influenced; rendered as text only.
  let {
    held,
    selectedId,
    onselect,
  }: { held: HeldDto[]; selectedId: string | null; onselect: (id: string) => void } = $props();
</script>

<ul class="held" data-testid="held-list" aria-label="Held requests">
  {#each held as h (h.id)}
    <li>
      <button
        type="button"
        class="row"
        class:sel={h.id === selectedId}
        data-testid="held-row"
        data-editable={h.editable}
        aria-current={h.id === selectedId ? 'true' : undefined}
        onclick={() => onselect(h.id)}
      >
        <span class="method">{h.method}</span>
        <span class="host">{h.host}</span>
        <span class="path">{h.path}</span>
        <span class="size">{h.size} B</span>
        {#if !h.editable}<span class="badge">not editable</span>{/if}
      </button>
    </li>
  {:else}
    <li class="empty" data-testid="held-empty">Nothing held.</li>
  {/each}
</ul>

<style>
  .held {
    list-style: none;
    margin: 0;
    padding: 0;
    border: 1px solid #8884;
    border-radius: 4px;
    max-height: 14rem;
    overflow: auto;
  }
  .row {
    display: flex;
    gap: 0.6rem;
    width: 100%;
    text-align: left;
    background: none;
    border: none;
    color: inherit;
    font: inherit;
    font-size: 0.85rem;
    padding: 0.3rem 0.5rem;
    cursor: pointer;
  }
  .row.sel {
    background: #26263a;
  }
  .method {
    font-weight: 600;
    min-width: 3.5rem;
  }
  .path {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .size {
    opacity: 0.7;
  }
  .badge {
    font-size: 0.7rem;
    border: 1px solid #b9770e;
    color: #e0a040;
    border-radius: 3px;
    padding: 0 0.3rem;
  }
  .empty {
    padding: 0.5rem;
    opacity: 0.7;
  }
</style>
