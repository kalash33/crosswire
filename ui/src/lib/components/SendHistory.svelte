<script lang="ts">
  import { historyKey, type SendRecord } from '../stores/repeater.svelte';
  import { clipText, humanBytes, timeShort } from '../format';
  import Badge from '../ui/Badge.svelte';
  import Button from '../ui/Button.svelte';
  import Icon from '../ui/Icon.svelte';

  // The send-history strip under the repeater tab bar: one entry per send,
  // newest first, as a horizontal tape. Clicking an entry restores it. Every
  // value shown (status, error, sizes) is wire or user data: text only.
  let {
    history,
    shown = -1,
    dirty = false,
    notice = null,
    canCompare = false,
    comparing = false,
    onrestore,
    onclear,
    oncompare,
  }: {
    history: SendRecord[];
    /** Index of the record whose response is on screen, or -1. */
    shown?: number;
    /** The editor holds unsent changes. */
    dirty?: boolean;
    notice?: string | null;
    canCompare?: boolean;
    comparing?: boolean;
    onrestore: (index: number) => void;
    onclear: () => void;
    oncompare: () => void;
  } = $props();

  // Collapsed state is a per-browser convenience shared by every tab.
  // Storage can be unavailable (private mode, blocked site data): never fatal.
  const KEY = 'crosswire.repeater.history-collapsed';
  function loadCollapsed(): boolean {
    try {
      return localStorage.getItem(KEY) === '1';
    } catch {
      return false;
    }
  }
  let collapsed = $state(loadCollapsed());
  function toggle(): void {
    collapsed = !collapsed;
    try {
      localStorage.setItem(KEY, collapsed ? '1' : '0');
    } catch {
      // storage unavailable: the state just does not persist
    }
  }

  const uid = $props.id();
  const listId = `send-history-list-${uid}`;

  function describe(r: SendRecord): string {
    const parts = [timeShort(r.at)];
    if (r.error) parts.push(`failed: ${r.error}`);
    else {
      if (r.status !== undefined) parts.push(`status ${r.status}`);
      if (r.size !== undefined) parts.push(humanBytes(r.size));
      if (r.ttfb_ms !== undefined) parts.push(`TTFB ${r.ttfb_ms} ms`);
    }
    if (r.truncated) parts.push('request not kept');
    return parts.join(', ');
  }

  // Roving arrow keys across the entries (Home/End too).
  let list = $state<HTMLElement>();
  function onKey(e: KeyboardEvent): void {
    const items = Array.from(list?.querySelectorAll<HTMLElement>('[data-testid="send-history-row"]') ?? []);
    const i = items.indexOf(document.activeElement as HTMLElement);
    if (i < 0) return;
    let next = -1;
    if (e.key === 'ArrowRight') next = Math.min(i + 1, items.length - 1);
    else if (e.key === 'ArrowLeft') next = Math.max(i - 1, 0);
    else if (e.key === 'Home') next = 0;
    else if (e.key === 'End') next = items.length - 1;
    if (next < 0) return;
    e.preventDefault();
    items[next].focus();
  }
</script>

<section class="send-history" class:collapsed data-testid="send-history" aria-label="Send history">
  <div class="bar">
    <button
      type="button"
      class="toggle"
      aria-expanded={!collapsed}
      aria-controls={listId}
      data-testid="send-history-toggle"
      onclick={toggle}
    >
      <Icon name={collapsed ? 'chevron-right' : 'chevron-down'} size={14} />
      <span class="title">History</span>
      <span class="count" data-testid="send-history-count">{history.length}</span>
    </button>
    {#if dirty && history.length > 0}
      <span class="dirty" data-testid="send-history-dirty">Unsent changes</span>
    {/if}
    {#if notice}
      <span class="notice" data-testid="send-history-notice" role="status">{notice}</span>
    {/if}
    <span class="spacer"></span>
    <Button
      size="sm"
      variant={comparing ? 'default' : 'ghost'}
      data-testid="compare-previous"
      disabled={!canCompare}
      aria-pressed={comparing}
      title={canCompare
        ? 'Diff the shown response against the previous one'
        : 'Needs two sends with a response'}
      onclick={oncompare}>Compare with previous</Button
    >
    <Button
      size="sm"
      variant="ghost"
      data-testid="send-history-clear"
      disabled={history.length === 0}
      onclick={onclear}>Clear</Button
    >
  </div>

  {#if !collapsed}
    <div class="tape" id={listId} bind:this={list} role="list">
      {#if history.length === 0}
        <p class="empty" data-testid="send-history-empty">Each send is kept here. Click one to load it back.</p>
      {/if}
      {#each history as r, i (historyKey(r))}
        <div role="listitem" class="item">
          <button
            type="button"
            class="entry"
            class:shown={i === shown}
            class:failed={!!r.error}
            data-testid="send-history-row"
            aria-current={i === shown ? 'true' : undefined}
            aria-label={`Restore send: ${describe(r)}`}
            title={describe(r)}
            onclick={() => onrestore(i)}
            onkeydown={onKey}
          >
            <span class="top">
              {#if r.error}
                <span class="err" data-testid="send-history-error">failed</span>
              {:else}
                <span data-testid="send-history-status"><Badge kind="status" value={r.status ?? '···'} /></span>
              {/if}
              <span class="time">{timeShort(r.at)}</span>
            </span>
            <span class="meta">
              {#if r.error}
                <span class="reason" data-testid="send-history-reason" title={r.error}>{clipText(r.error)}</span>
              {:else}
                <span data-testid="send-history-size">{r.size !== undefined ? humanBytes(r.size) : '–'}</span>
                <span data-testid="send-history-ttfb">{r.ttfb_ms !== undefined ? `${r.ttfb_ms} ms` : '–'}</span>
              {/if}
              {#if r.truncated}<span class="cut" title="The request was too large to keep">resp only</span>{/if}
            </span>
          </button>
        </div>
      {/each}
    </div>
  {/if}
</section>

<style>
  .send-history {
    flex: none;
    display: flex;
    flex-direction: column;
    border-bottom: 1px solid var(--border);
    background: var(--bg);
    min-width: 0;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    min-height: 32px;
    padding: 0 var(--s-3) 0 var(--s-2);
    min-width: 0;
  }
  .toggle {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 24px;
    padding: 0 6px;
    font: inherit;
    font-size: var(--fs-sm);
    color: var(--fg-muted);
    background: none;
    border: none;
    border-radius: var(--r-1);
    cursor: pointer;
  }
  .toggle:hover {
    color: var(--fg);
    background: var(--bg-hover);
  }
  .toggle:focus-visible,
  .entry:focus-visible {
    outline: 2px solid var(--focus);
    outline-offset: -2px;
  }
  .title {
    font-weight: 600;
    color: var(--fg);
  }
  .count {
    min-width: 18px;
    padding: 0 5px;
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
    text-align: center;
    color: var(--fg-muted);
    background: var(--chip-bg);
    border-radius: var(--r-pill);
  }
  .dirty {
    font-size: var(--fs-xs);
    color: var(--warn);
  }
  .notice {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--fs-sm);
    color: var(--info);
  }
  .spacer {
    flex: 1;
  }
  .tape {
    display: flex;
    gap: 0;
    padding: 0 var(--s-3) var(--s-2);
    overflow-x: auto;
    scrollbar-width: thin;
    min-width: 0;
  }
  .empty {
    margin: 0;
    padding: var(--s-1) var(--s-2) 0;
    font-size: var(--fs-sm);
    color: var(--fg-faint);
  }
  .item {
    flex: none;
    display: flex;
  }
  /* Entries sit on a shared baseline rule, like ticks on a timeline; the
     one whose response is on screen lifts the accent onto that rule. */
  .entry {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 7.5rem;
    max-width: 12rem;
    padding: 6px 10px 7px;
    font: inherit;
    text-align: left;
    color: var(--fg);
    background: none;
    border: none;
    border-bottom: 2px solid var(--border);
    cursor: pointer;
  }
  .item + .item .entry {
    box-shadow: inset 1px 0 0 var(--border-faint);
  }
  .entry:hover {
    background: var(--bg-hover);
  }
  .entry.shown {
    background: var(--accent-soft);
    border-bottom-color: var(--accent);
  }
  .entry.failed {
    border-bottom-color: var(--danger);
  }
  .top {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .time {
    font-family: var(--font-mono);
    font-size: var(--fs-sm);
    font-variant-numeric: tabular-nums;
  }
  .meta {
    display: flex;
    gap: 10px;
    min-width: 0;
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
    color: var(--fg-muted);
  }
  .err {
    height: 18px;
    padding: 0 6px;
    font-size: var(--fs-xs);
    font-weight: 600;
    line-height: 18px;
    color: var(--danger);
    box-shadow: inset 0 0 0 1px currentColor;
    border-radius: var(--r-1);
  }
  /* A failure reason wraps (clipped to ~80 characters, full text in its
     title) instead of being cut mid-word on one line. */
  .entry.failed {
    max-width: 16rem;
  }
  .reason {
    min-width: 0;
    white-space: normal;
    overflow-wrap: anywhere;
    color: var(--danger-fg);
  }
  .cut {
    color: var(--warn);
  }
</style>
