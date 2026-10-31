<script lang="ts">
  // A "Copy" button that opens the copy Menu for one exchange (curl, curl
  // redacted, raw, raw redacted, URL). With `chords`, it also owns the
  // copy-as-curl / copy-raw keymap actions (both UNREDACTED, for replay) while
  // mounted; with `showStatus`, it renders the shared `copy-status` line.
  import { onMount } from 'svelte';
  import Button from '../ui/Button.svelte';
  import Menu from '../ui/Menu.svelte';
  import { keymap } from '../keymap';
  import { copyExchange, copyItems, copyStatus } from '../copy-actions.svelte';
  import type { UrlSource } from '../copy-url';

  let {
    id,
    row = null,
    testid = 'copy-menu',
    chords = false,
    showStatus = false,
    size = 'sm',
  }: {
    id: string;
    row?: UrlSource | null;
    testid?: string;
    chords?: boolean;
    showStatus?: boolean;
    size?: 'sm' | 'md';
  } = $props();

  let open = $state(false);
  let wrap = $state<HTMLElement>();
  // The trigger <button> itself, so the Menu returns focus to it on close.
  let anchor = $state<HTMLElement | null>(null);
  const items = $derived(copyItems(id, row));

  onMount(() => {
    anchor = wrap?.querySelector('button') ?? null;
    if (!chords) return;
    const offCurl = keymap.register('copy-as-curl', () => void copyExchange(id, 'curl', false));
    const offRaw = keymap.register('copy-raw', () => void copyExchange(id, 'raw', false));
    return () => {
      offCurl();
      offRaw();
    };
  });
</script>

<span class="copy" bind:this={wrap}>
  <Button
    {size}
    icon="copy"
    data-testid={testid}
    aria-haspopup="menu"
    aria-expanded={open}
    onclick={() => (open = !open)}>Copy</Button
  >
  {#if showStatus && copyStatus.text}
    <span class="status" data-testid="copy-status" role="status">{copyStatus.text}</span>
  {/if}
</span>
<Menu {items} {open} {anchor} label="Copy" testid={`${testid}-items`} onclose={() => (open = false)} />

<style>
  .copy {
    display: inline-flex;
    align-items: center;
    gap: var(--s-2);
    min-width: 0;
  }
  .status {
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    white-space: nowrap;
  }
</style>
