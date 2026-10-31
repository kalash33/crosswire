<script lang="ts">
  import type { HTMLButtonAttributes } from 'svelte/elements';
  import Icon from './Icon.svelte';
  import type { IconName } from './icons';

  type Props = Omit<HTMLButtonAttributes, 'class' | 'aria-label' | 'title' | 'children'> & {
    icon: IconName;
    /** Required accessible name; also the tooltip. */
    label: string;
    /** Shortcut appended to the tooltip, e.g. "Ctrl+T". */
    kbd?: string;
    size?: 'sm' | 'md';
    active?: boolean;
    class?: string;
  };

  let {
    icon,
    label,
    kbd,
    size = 'md',
    active = false,
    type = 'button',
    class: klass = '',
    ...rest
  }: Props = $props();

  const tip = $derived(kbd ? `${label} (${kbd})` : label);
</script>

<button {...rest} {type} class="icon-btn {size} {klass}" class:active aria-label={label} title={tip}>
  <Icon name={icon} size={size === 'sm' ? 14 : 16} />
</button>

<style>
  .icon-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: var(--control-h-md);
    height: var(--control-h-md);
    min-height: 0;
    padding: 0;
    color: var(--fg-muted);
    background: transparent;
    border: 1px solid transparent;
    border-radius: var(--r-1);
    cursor: pointer;
  }
  .icon-btn.sm {
    width: var(--control-h-sm);
    height: var(--control-h-sm);
  }
  .icon-btn:hover:not(:disabled) {
    color: var(--fg);
    background: var(--bg-hover);
  }
  .icon-btn.active {
    color: var(--accent);
    background: var(--accent-soft);
  }
  .icon-btn:disabled {
    opacity: 0.45;
    cursor: default;
  }
</style>
