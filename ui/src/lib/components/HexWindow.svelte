<script lang="ts">
  // The Hex tab: HexPane over one 32 KiB (HEX_PAGE) window of the bytes at a time, with
  // previous/next paging, so a multi-megabyte body never becomes one DOM row
  // per 16 bytes of the whole buffer (U1 Review Focus 2).
  import HexPane from '../editor/HexPane.svelte';
  import Button from '../ui/Button.svelte';
  import { hexPage } from '../editor/hex-window';
  import { humanBytes } from '../format';

  let { bytes, testid = 'hex-window' }: { bytes: Uint8Array; testid?: string } = $props();

  let page = $state(0);
  const win = $derived(hexPage(bytes.length, page));
  const slice = $derived(bytes.subarray(win.start, win.end));
  const hex8 = (n: number) => n.toString(16).padStart(8, '0');
</script>

<div class="hexwin" data-testid={testid}>
  {#if win.pages > 1}
    <div class="pager">
      <Button
        size="sm"
        variant="ghost"
        icon="chevron-right"
        class="flip"
        data-testid="hex-prev"
        disabled={win.page === 0}
        onclick={() => (page = win.page - 1)}>Previous</Button
      >
      <span class="pos" data-testid="hex-page"
        title={`${bytes.length} bytes (${humanBytes(bytes.length)})`}
        >{hex8(win.start)}–{hex8(Math.max(win.start, win.end - 1))}, page {win.page + 1} of {win.pages}</span
      >
      <Button
        size="sm"
        variant="ghost"
        icon="chevron-right"
        data-testid="hex-next"
        disabled={win.page >= win.pages - 1}
        onclick={() => (page = win.page + 1)}>Next</Button
      >
    </div>
  {/if}
  <div class="dump"><HexPane bytes={slice} base={win.start} /></div>
</div>

<style>
  .hexwin {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
    min-width: 0;
  }
  /* A row is ~75 columns: a smaller face, and horizontal scroll inside the
     window (never clipping the ASCII gutter) when the pane is narrower. */
  .dump {
    overflow-x: auto;
    min-width: 0;
  }
  .dump :global(.hex-pane) {
    width: max-content;
    min-width: 100%;
    font-size: var(--fs-xs);
  }
  /* Tighter gutters: a 16-byte row fits in ~430px (the 1280px detail pane). */
  .dump :global(.hex-row) {
    gap: var(--s-2);
  }
  .dump :global(.hex-cells) {
    gap: 0.25em;
  }
  .pager {
    display: flex;
    align-items: center;
    gap: var(--s-2);
  }
  .pager :global(.flip svg) {
    transform: scaleX(-1);
  }
  .pos {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    color: var(--fg-muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }
</style>
