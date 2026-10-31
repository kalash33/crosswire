<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { HTMLButtonAttributes } from 'svelte/elements';
  import Icon from './Icon.svelte';
  import Kbd from './Kbd.svelte';
  import type { IconName } from './icons';

  type Props = Omit<HTMLButtonAttributes, 'class'> & {
    variant?: 'primary' | 'default' | 'ghost' | 'danger';
    size?: 'sm' | 'md';
    icon?: IconName;
    /** Shortcut hint shown inside the button, e.g. "Ctrl+Space". */
    kbd?: string;
    class?: string;
    children?: Snippet;
  };

  let {
    variant = 'default',
    size = 'md',
    icon,
    kbd,
    type = 'button',
    class: klass = '',
    children,
    ...rest
  }: Props = $props();
</script>

<button {...rest} {type} class="btn {variant} {size} {klass}">
  {#if icon}<Icon name={icon} size={size === 'sm' ? 14 : 16} />{/if}
  {#if children}<span class="label">{@render children()}</span>{/if}
  {#if kbd}<Kbd keys={kbd} />{/if}
</button>

<style>
  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    height: var(--control-h-md);
    min-height: 0;
    padding: 0 10px;
    font: inherit;
    font-size: var(--fs-md);
    font-weight: 500;
    white-space: nowrap;
    color: var(--fg);
    background: var(--bg-elev);
    border: 1px solid var(--border-strong);
    border-radius: var(--r-1);
    cursor: pointer;
  }
  .btn.sm {
    height: var(--control-h-sm);
    padding: 0 var(--s-2);
    font-size: var(--fs-sm);
    gap: var(--s-1);
  }
  .btn:hover:not(:disabled) {
    background: var(--bg-hover);
  }
  .btn:active:not(:disabled) {
    background: var(--bg-active);
  }
  .btn.primary {
    color: var(--accent-fg);
    background: var(--accent);
    border-color: var(--accent);
  }
  .btn.primary:hover:not(:disabled) {
    background: var(--accent-hover);
    border-color: var(--accent-hover);
  }
  .btn.primary :global(.kbd) {
    color: var(--accent-fg);
    background: transparent;
    border-color: currentColor;
    opacity: 0.75;
  }
  .btn.ghost {
    background: transparent;
    border-color: transparent;
    color: var(--fg-muted);
  }
  .btn.ghost:hover:not(:disabled) {
    color: var(--fg);
    background: var(--bg-hover);
  }
  .btn.danger {
    color: var(--danger-fg);
    background: var(--danger-bg);
    border-color: var(--danger);
  }
  .btn.danger:hover:not(:disabled) {
    color: var(--fg-on-strong);
    background: var(--danger);
  }
  .btn:disabled {
    opacity: 0.45;
    cursor: default;
  }
  .label {
    display: inline-flex;
    align-items: center;
  }
</style>
