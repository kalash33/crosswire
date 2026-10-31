<script lang="ts">
  // The single toast outlet; mounted once by the shell.
  import Icon from './Icon.svelte';
  import type { IconName } from './icons';
  import { toasts, type ToastKind } from './toasts.svelte';

  const ICON: Record<ToastKind, IconName> = { info: 'check', success: 'check', error: 'warning' };
</script>

<div class="toasts" role="status" aria-live="polite" data-testid="toasts">
  {#each toasts.items as t (t.id)}
    <div class="toast {t.kind}" data-testid="toast" data-kind={t.kind}>
      <span class="mark"><Icon name={ICON[t.kind]} size={14} /></span>
      <span class="text">{t.text}</span>
      <button type="button" class="x" aria-label="Dismiss notification" title="Dismiss" onclick={() => toasts.dismiss(t.id)}>
        <Icon name="close" size={12} />
      </button>
    </div>
  {/each}
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
