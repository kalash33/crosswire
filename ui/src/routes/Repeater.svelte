<script lang="ts">
  import { onMount } from 'svelte';
  import { repeater } from '../lib/stores/repeater.svelte';
  import RepeaterTab from '../lib/components/RepeaterTab.svelte';

  // Load persisted tab drafts on mount. If none survive, open a fresh tab so the
  // view always presents at least one tab to work in.
  onMount(() => {
    void (async () => {
      await repeater.loadTabs();
      if (repeater.tabs.length === 0) repeater.newTab();
    })();
  });

  function tabLabel(host: string, i: number): string {
    return host.trim() ? host.trim() : `Tab ${i + 1}`;
  }
</script>

<div class="repeater" data-testid="repeater-view">
  <div class="tabstrip" data-testid="repeater-tabs" role="tablist">
    {#each repeater.tabs as tab, i (tab.id)}
      <div class="tab" class:active={tab.id === repeater.activeTabId} data-testid="repeater-tab">
        <button
          type="button"
          class="label"
          role="tab"
          aria-selected={tab.id === repeater.activeTabId}
          data-testid="repeater-tab-select"
          onclick={() => repeater.selectTab(tab.id)}
        >
          {tabLabel(tab.conn.host, i)}
        </button>
        <button
          type="button"
          class="close"
          aria-label="close tab"
          data-testid="close-tab"
          onclick={() => void repeater.closeTab(tab.id)}>×</button
        >
      </div>
    {/each}
    <button type="button" class="new" data-testid="new-tab" onclick={() => repeater.newTab()}
      >+ New</button
    >
  </div>

  {#if repeater.loadError}
    <p class="error" data-testid="repeater-load-error" role="alert">{repeater.loadError}</p>
  {/if}

  {#if repeater.activeTab}
    {#key repeater.activeTab.id}
      <RepeaterTab tab={repeater.activeTab} />
    {/key}
  {:else}
    <p class="placeholder" data-testid="repeater-empty">No tab open.</p>
  {/if}
</div>

<style>
  .repeater {
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
    min-width: 0;
  }
  .tabstrip {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.3rem;
    border-bottom: 1px solid #8883;
    padding-bottom: 0.3rem;
  }
  .tab {
    display: inline-flex;
    align-items: center;
    border: 1px solid #3a3a4a;
    border-radius: 4px;
    overflow: hidden;
  }
  .tab.active {
    border-color: #6a6aaa;
    background: #26263a;
  }
  .tab .label {
    background: none;
    border: none;
    color: inherit;
    padding: 0.25rem 0.5rem;
    font-size: 0.78rem;
    cursor: pointer;
    max-width: 14rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tab .close {
    background: none;
    border: none;
    color: inherit;
    cursor: pointer;
    font-size: 0.9rem;
    line-height: 1;
    padding: 0 0.4rem;
    opacity: 0.7;
  }
  .tab .close:hover {
    opacity: 1;
  }
  .new {
    background: #22222e;
    color: #cfd0ff;
    border: 1px solid #3a3a4a;
    border-radius: 4px;
    padding: 0.25rem 0.5rem;
    font-size: 0.78rem;
    cursor: pointer;
  }
  .error {
    color: #ff9b9b;
    margin: 0;
    font-size: 0.85rem;
  }
  .placeholder {
    opacity: 0.6;
    padding: 0.75rem;
  }
</style>
