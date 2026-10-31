<script lang="ts">
  import type { Snippet } from 'svelte';
  import Icon from './Icon.svelte';
  import type { IconName } from './icons';

  let {
    icon,
    title,
    body,
    actions,
    testid,
  }: {
    icon?: IconName;
    title: string;
    body?: string;
    actions?: Snippet;
    testid?: string;
  } = $props();
</script>

<div class="empty-state" data-testid={testid}>
  {#if icon}<span class="glyph"><Icon name={icon} size={20} /></span>{/if}
  <p class="title">{title}</p>
  {#if body}<p class="body">{body}</p>{/if}
  {#if actions}<div class="actions">{@render actions()}</div>{/if}
</div>

<style>
  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--s-2);
    padding: var(--s-6) var(--s-4);
    text-align: center;
    color: var(--fg-muted);
  }
  .glyph {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 40px;
    height: 40px;
    color: var(--fg-muted);
    border: 1px solid var(--border);
    border-radius: var(--r-2);
    background: var(--bg-elev);
  }
  .title {
    margin: var(--s-1) 0 0;
    font-size: var(--fs-lg);
    font-weight: 600;
    color: var(--fg);
  }
  .body {
    margin: 0;
    max-width: 46ch;
    font-size: var(--fs-md);
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: var(--s-2);
    margin-top: var(--s-2);
  }
</style>
