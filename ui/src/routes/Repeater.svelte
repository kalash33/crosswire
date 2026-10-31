<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { repeater } from '../lib/stores/repeater.svelte';
  import { keymap } from '../lib/keymap';
  import RepeaterTab from '../lib/components/RepeaterTab.svelte';
  import { toasts } from '../lib/ui/toasts.svelte';
  import Button from '../lib/ui/Button.svelte';
  import EmptyState from '../lib/ui/EmptyState.svelte';
  import Icon from '../lib/ui/Icon.svelte';

  // Cycle the active tab by a signed step (wraps). Ctrl+Shift+] / [ in Burp.
  function cycleTab(step: number): void {
    const tabs = repeater.tabs;
    if (tabs.length < 2) return;
    const i = tabs.findIndex((t) => t.id === repeater.activeTabId);
    const next = tabs[(((i < 0 ? 0 : i) + step) % tabs.length + tabs.length) % tabs.length];
    if (next) repeater.selectTab(next.id);
  }

  // Send a tab; a failure is shown in the response pane and raised as a toast.
  async function sendTab(id: string): Promise<void> {
    await repeater.send(id);
    const t = repeater.tabs.find((x) => x.id === id);
    if (t?.error) toasts.push({ kind: 'error', text: `Send failed: ${t.error}` });
  }

  // Load persisted tab drafts on mount. If none survive, open a fresh tab so the
  // view always presents at least one tab to work in. The send / tab-cycle
  // chords are owned by the repeater while it is mounted (Burp: Ctrl+Space sends
  // even from inside the editor), and torn down on unmount.
  onMount(() => {
    void (async () => {
      await repeater.loadTabs();
      if (repeater.tabs.length === 0) repeater.newTab();
    })();
    const offs = [
      keymap.register('send', () => {
        if (repeater.activeTabId) void sendTab(repeater.activeTabId);
      }),
      keymap.register('tab-next', () => cycleTab(1)),
      keymap.register('tab-prev', () => cycleTab(-1)),
    ];
    return () => offs.forEach((off) => off());
  });

  // ---- Tab strip keyboard: roving focus (Left/Right wrap, Home/End), F2 renames ----
  let tabList = $state<HTMLElement>();

  async function onTabKey(e: KeyboardEvent, id: string): Promise<void> {
    if (e.key === 'F2') {
      e.preventDefault();
      void startRename(id);
      return;
    }
    const tabs = repeater.tabs;
    const i = tabs.findIndex((t) => t.id === id);
    let next = -1;
    if (e.key === 'ArrowRight') next = (i + 1) % tabs.length;
    else if (e.key === 'ArrowLeft') next = (i - 1 + tabs.length) % tabs.length;
    else if (e.key === 'Home') next = 0;
    else if (e.key === 'End') next = tabs.length - 1;
    if (next < 0 || i < 0) return;
    e.preventDefault();
    repeater.selectTab(tabs[next].id);
    await tick();
    tabList?.querySelectorAll<HTMLElement>('[role="tab"]')[next]?.focus();
  }

  // ---- Inline rename (double-click or F2; Enter commits, Escape cancels) ----
  let editingId = $state<string | null>(null);
  let draftTitle = $state('');
  let renameInput = $state<HTMLInputElement>();

  async function startRename(id: string): Promise<void> {
    const t = repeater.tabs.find((x) => x.id === id);
    if (!t) return;
    repeater.selectTab(id);
    draftTitle = repeater.titleFor(t);
    editingId = id;
    await tick();
    renameInput?.focus();
    renameInput?.select();
  }

  function commitRename(): void {
    if (editingId === null) return;
    const id = editingId;
    editingId = null;
    const t = repeater.tabs.find((x) => x.id === id);
    // Committing the unchanged derived title is not an explicit rename.
    if (t && draftTitle.trim() !== repeater.titleFor(t)) repeater.renameTab(id, draftTitle);
  }

  function onRenameKey(e: KeyboardEvent): void {
    if (e.key === 'Enter') {
      e.preventDefault();
      commitRename();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      editingId = null;
    }
  }
</script>

<div class="repeater" data-testid="repeater-view">
  <div class="tabstrip">
    <div class="tabs" data-testid="repeater-tabs" role="tablist" aria-label="Repeater tabs" bind:this={tabList}>
      {#each repeater.tabs as tab (tab.id)}
        {@const title = repeater.titleFor(tab)}
        {@const active = tab.id === repeater.activeTabId}
        <div class="tab" class:active data-testid="repeater-tab">
          {#if editingId === tab.id}
            <input
              class="rename"
              data-testid="repeater-tab-rename"
              aria-label="Tab name"
              bind:this={renameInput}
              bind:value={draftTitle}
              onkeydown={onRenameKey}
              onblur={commitRename}
              maxlength="120"
              spellcheck="false"
            />
          {:else}
            <button
              type="button"
              class="label"
              role="tab"
              aria-selected={active}
              tabindex={active ? 0 : -1}
              title="{title} (double-click to rename)"
              data-testid="repeater-tab-select"
              onclick={() => repeater.selectTab(tab.id)}
              ondblclick={() => void startRename(tab.id)}
              onkeydown={(e) => onTabKey(e, tab.id)}
            >
              {#if tab.sending}<span class="dot" aria-hidden="true"></span>{/if}
              <span class="text">{title}</span>
            </button>
          {/if}
          <button
            type="button"
            class="close"
            aria-label="Close {title}"
            title="Close tab"
            data-testid="close-tab"
            onclick={() => void repeater.closeTab(tab.id)}><Icon name="close" size={12} /></button
          >
        </div>
      {/each}
    </div>
    <Button size="sm" variant="ghost" icon="plus" data-testid="new-tab" title="New tab" onclick={() => repeater.newTab()}
      >New</Button
    >
  </div>

  {#if repeater.loadError}
    <p class="error" data-testid="repeater-load-error" role="alert">{repeater.loadError}</p>
  {/if}

  {#if repeater.activeTab}
    {#key repeater.activeTab.id}
      <RepeaterTab tab={repeater.activeTab} onsend={(id) => void sendTab(id)} />
    {/key}
  {:else}
    <EmptyState icon="repeater" title="No tab open" body="Open a tab to build and send a request." testid="repeater-empty">
      {#snippet actions()}
        <Button variant="primary" icon="plus" onclick={() => repeater.newTab()}>New tab</Button>
      {/snippet}
    </EmptyState>
  {/if}
</div>

<style>
  .repeater {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    min-width: 0;
  }
  .tabstrip {
    flex: none;
    display: flex;
    align-items: flex-end;
    gap: var(--s-2);
    padding: var(--s-2) var(--s-3) 0;
    border-bottom: 1px solid var(--border);
    background: var(--bg-elev);
    min-width: 0;
  }
  .tabs {
    display: flex;
    align-items: flex-end;
    gap: 2px;
    min-width: 0;
    overflow-x: auto;
    scrollbar-width: thin;
  }
  .tab {
    flex: none;
    position: relative;
    display: inline-flex;
    align-items: center;
    height: 30px;
    max-width: 16rem;
    border: 1px solid transparent;
    border-bottom: none;
    border-radius: var(--r-2) var(--r-2) 0 0;
    color: var(--fg-muted);
  }
  .tab:hover {
    background: var(--bg-hover);
    color: var(--fg);
  }
  .tab.active {
    background: var(--bg);
    border-color: var(--border);
    color: var(--fg);
    margin-bottom: -1px;
    height: 31px;
  }
  .tab.active::before {
    content: '';
    position: absolute;
    left: -1px;
    right: -1px;
    top: -1px;
    height: 2px;
    border-radius: var(--r-2) var(--r-2) 0 0;
    background: var(--accent);
  }
  .label {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    height: 100%;
    padding: 0 4px 0 10px;
    font: inherit;
    font-family: var(--font-mono);
    font-size: var(--fs-sm);
    color: inherit;
    background: none;
    border: none;
    cursor: pointer;
  }
  .label:focus-visible,
  .close:focus-visible {
    outline: 2px solid var(--focus);
    outline-offset: -2px;
    border-radius: var(--r-1);
  }
  .text {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dot {
    flex: none;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--accent);
  }
  .rename {
    width: 12rem;
    height: 22px;
    margin: 0 4px 0 6px;
    padding: 0 6px;
    font: inherit;
    font-family: var(--font-mono);
    font-size: var(--fs-sm);
    color: var(--fg);
    background: var(--bg-sunken);
    border: 1px solid var(--focus);
    border-radius: var(--r-1);
  }
  .close {
    flex: none;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 20px;
    height: 20px;
    margin-right: 4px;
    padding: 0;
    color: var(--fg-faint);
    background: none;
    border: none;
    border-radius: var(--r-1);
    cursor: pointer;
  }
  .close:hover {
    color: var(--fg);
    background: var(--bg-active);
  }
  .tabstrip > :global(.btn) {
    flex: none;
    margin-bottom: 3px;
  }
  .error {
    margin: 0;
    padding: var(--s-2) var(--s-3);
    font-size: var(--fs-sm);
    color: var(--danger);
  }
</style>
