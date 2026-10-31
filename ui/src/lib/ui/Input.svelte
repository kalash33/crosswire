<script lang="ts">
  import type { HTMLInputAttributes } from 'svelte/elements';

  type Props = Omit<HTMLInputAttributes, 'class' | 'size'> & {
    value?: string | number | null;
    size?: 'sm' | 'md';
    mono?: boolean;
    class?: string;
  };

  let { value = $bindable(), size = 'md', mono = false, class: klass = '', ...rest }: Props = $props();
</script>

<input {...rest} bind:value class="input {size} {klass}" class:mono />

<style>
  .input {
    height: var(--control-h-md);
    min-height: 0;
    padding: 0 var(--s-2);
    font: inherit;
    font-size: var(--fs-md);
    color: var(--fg);
    background: var(--bg-sunken);
    border: 1px solid var(--border);
    border-radius: var(--r-1);
    min-width: 0;
  }
  .input.sm {
    height: var(--control-h-sm);
    font-size: var(--fs-sm);
    padding: 0 6px;
  }
  .input.mono {
    font-family: var(--font-mono);
  }
  .input:hover {
    border-color: var(--border-strong);
  }
  .input:focus-visible {
    outline: 2px solid var(--focus);
    outline-offset: -1px;
    border-color: transparent;
  }
  .input::placeholder {
    color: var(--fg-faint);
  }
  .input:disabled {
    opacity: 0.5;
  }
</style>
