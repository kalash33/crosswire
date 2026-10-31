<script lang="ts">
  import type { HeldDto } from '../api/intercept';
  import Badge from '../ui/Badge.svelte';
  import Button from '../ui/Button.svelte';
  import EmptyState from '../ui/EmptyState.svelte';

  // Held rows: host/method/path are attacker-influenced; rendered as text only.
  let {
    held,
    selectedId,
    onselect,
    enabled = true,
    onenable,
  }: {
    held: HeldDto[];
    selectedId: string | null;
    onselect: (id: string) => void;
    enabled?: boolean;
    onenable?: () => void;
  } = $props();
</script>

{#if held.length === 0}
  {#if enabled}
    <EmptyState
      testid="held-empty"
      icon="intercept"
      title="Nothing held"
      body="Requests appear here while intercept is on and a request matches your rules."
    />
  {:else}
    <EmptyState
      testid="held-empty"
      icon="pause"
      title="Intercept is off"
      body="Turn it on to hold requests here before they reach the server."
    >
      {#snippet actions()}
        <Button variant="primary" onclick={() => onenable?.()}>Turn on</Button>
      {/snippet}
    </EmptyState>
  {/if}
{/if}
<ul class="held" data-testid="held-list" aria-label="Held requests" class:none={held.length === 0}>
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
        onfocus={() => onselect(h.id)}
      >
        <Badge kind="method" value={h.method} />
        <span class="host">{h.host}</span>
        <span class="path">{h.path}</span>
        <span class="size">{h.size} B</span>
        {#if !h.editable}<Badge kind="state" value="not editable" />{/if}
      </button>
    </li>
  {/each}
</ul>

<style>
  .held {
    list-style: none;
    margin: 0;
    padding: 0;
    border: 1px solid var(--border);
    border-radius: var(--r-1);
    max-height: 14rem;
    overflow: auto;
    background: var(--bg-sunken);
  }
  .held.none {
    display: none;
  }
  .row {
    display: flex;
    align-items: center;
    gap: var(--s-3);
    width: 100%;
    text-align: left;
    background: none;
    border: none;
    border-radius: 0;
    color: inherit;
    font: inherit;
    font-size: var(--fs-md);
    padding: var(--s-1) var(--s-3);
    cursor: pointer;
    min-width: 0;
  }
  .row:hover {
    background: var(--bg-hover);
  }
  .row.sel {
    background: var(--selection);
  }
  .host {
    color: var(--fg-muted);
    white-space: nowrap;
  }
  .path {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--font-mono);
  }
  .size {
    color: var(--fg-faint);
    font-variant-numeric: tabular-nums;
  }
</style>
