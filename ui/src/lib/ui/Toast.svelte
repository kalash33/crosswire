<script lang="ts">
  // The single toast outlet; mounted once by the shell.
  import Icon from './Icon.svelte';
  import type { IconName } from './icons';
  import { toasts, type ToastItem, type ToastKind } from './toasts.svelte';

  const ICON: Record<ToastKind, IconName> = { info: 'info', success: 'check', error: 'warning' };

  const errors = $derived(toasts.items.filter((t) => t.kind === 'error'));
  const notices = $derived(toasts.items.filter((t) => t.kind !== 'error'));
</script>

{#snippet toast(t: ToastItem, role: 'alert' | undefined)}
  <div class="toast {t.kind}" {role} data-testid="toast" data-kind={t.kind}>
    <span class="mark"><Icon name={ICON[t.kind]} size={14} /></span>
    <span class="text">{t.text}</span>
    <button type="button" class="x" aria-label="Dismiss notification" title="Dismiss" onclick={() => toasts.dismiss(t.id)}>
      <Icon name="close" size={12} />
    </button>
  </div>
{/snippet}

<!-- Errors are announced assertively (each one role="alert"); info and
     success share one polite status region. Separate regions, never nested. -->
<div class="toasts" data-testid="toasts">
  <div class="region">
    {#each errors as t (t.id)}{@render toast(t, 'alert')}{/each}
  </div>
  <div class="region" role="status" aria-live="polite">
    {#each notices as t (t.id)}{@render toast(t, undefined)}{/each}
  </div>
</div>

<style>
  .toasts {
    position: fixed;
    right: var(--s-4);
    bottom: calc(var(--statusbar-h) + var(--s-3));
    z-index: var(--z-toast);
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: var(--s-2);
    pointer-events: none;
  }
  .region {
    display: contents;
  }
  .toast {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    max-width: min(420px, calc(100vw - 2 * var(--s-4)));
    padding: 6px 6px 6px 10px;
    font-size: var(--fs-md);
    color: var(--fg);
    background: var(--bg-overlay);
    border: 1px solid var(--border-strong);
    border-left: 3px solid var(--info);
    border-radius: var(--r-2);
    box-shadow: var(--shadow-pop);
    pointer-events: auto;
  }
  .toast.success {
    border-left-color: var(--ok);
  }
  .toast.error {
    border-left-color: var(--danger);
  }
  .mark {
    display: inline-flex;
    color: var(--info);
  }
  .success .mark {
    color: var(--ok);
  }
  .error .mark {
    color: var(--danger);
  }
  .text {
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .x {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 20px;
    height: 20px;
    min-height: 0;
    padding: 0;
    color: var(--fg-muted);
    background: transparent;
    border: none;
    border-radius: var(--r-1);
    cursor: pointer;
  }
  .x:hover {
    color: var(--fg);
    background: var(--bg-hover);
  }
</style>
