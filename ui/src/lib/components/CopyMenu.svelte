<script lang="ts">
  import { onMount } from 'svelte';
  import { curl, raw } from '../api/copy';
  import { bytesToLatin1 } from '../editor/encoding';
  import { keymap } from '../keymap';

  // Shared copy control for any view showing a recorded exchange. Every copy
  // goes through the audited server routes, then the async Clipboard API; the
  // text is never inserted into the DOM. The keymap chords copy UNREDACTED (for
  // replay); the "(redacted)" buttons are the share-safe variants.
  let { id }: { id: string } = $props();

  let status = $state<string | null>(null);

  async function copy(kind: 'curl' | 'raw', redacted: boolean): Promise<void> {
    const target = id;
    try {
      const text =
        kind === 'curl'
          ? await curl(target, { redacted })
          : bytesToLatin1(await raw(target, { redacted }));
      await navigator.clipboard.writeText(text);
      status = `Copied ${kind}${redacted ? ' (redacted)' : ''}.`;
    } catch (e) {
      status = `Copy failed: ${e instanceof Error ? e.message : 'clipboard unavailable'}`;
    }
  }

  onMount(() => {
    const offCurl = keymap.register('copy-as-curl', () => void copy('curl', false));
    const offRaw = keymap.register('copy-raw', () => void copy('raw', false));
    return () => {
      offCurl();
      offRaw();
    };
  });
</script>

<div class="copy-menu" data-testid="copy-menu">
  <button type="button" data-testid="copy-curl" onclick={() => void copy('curl', false)}
    >Copy as curl</button
  >
  <button
    type="button"
    data-testid="copy-curl-redacted"
    onclick={() => void copy('curl', true)}>Copy as curl (redacted)</button
  >
  <button type="button" data-testid="copy-request-raw" onclick={() => void copy('raw', false)}
    >Copy raw</button
  >
  <button
    type="button"
    data-testid="copy-request-raw-redacted"
    onclick={() => void copy('raw', true)}>Copy raw (redacted)</button
  >
  {#if status}<span class="status" data-testid="copy-status" role="status">{status}</span>{/if}
</div>

<style>
  .copy-menu {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.4rem;
  }
  .status {
    font-size: 0.78rem;
    opacity: 0.8;
  }
</style>
