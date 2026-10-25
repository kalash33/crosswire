<script lang="ts">
  // A classic hex dump of the SAME bytes the editor holds: offset, up to 16 hex
  // columns, and a Latin-1/ASCII gutter. It reads the raw Uint8Array directly
  // (never a lossy string), so it matches the editor byte for byte.
  let { bytes = new Uint8Array() }: { bytes?: Uint8Array } = $props();

  const BYTES_PER_ROW = 16;

  function hx(b: number): string {
    return b.toString(16).padStart(2, '0');
  }

  const rows = $derived.by(() => {
    const out: { offset: string; cells: string[]; ascii: string }[] = [];
    for (let off = 0; off < bytes.length; off += BYTES_PER_ROW) {
      const slice = bytes.subarray(off, off + BYTES_PER_ROW);
      const cells = Array.from(slice, hx);
      let ascii = '';
      for (const b of slice) ascii += b >= 0x20 && b <= 0x7e ? String.fromCharCode(b) : '.';
      out.push({ offset: off.toString(16).padStart(8, '0'), cells, ascii });
    }
    return out;
  });

  // Flat, space-free hex of every byte — a single value the e2e test can assert
  // against to prove the pane matches the input exactly.
  const allHex = $derived(Array.from(bytes, hx).join(''));
</script>

<div class="hex-pane" data-testid="hex-pane" data-hex={allHex}>
  {#if rows.length === 0}
    <div class="hex-empty">(empty)</div>
  {:else}
    {#each rows as row (row.offset)}
      <div class="hex-row">
        <span class="hex-offset">{row.offset}</span>
        <span class="hex-cells">
          {#each row.cells as cell, i (i)}<span class="hex-cell">{cell}</span>{/each}
        </span>
        <span class="hex-ascii">{row.ascii}</span>
      </div>
    {/each}
  {/if}
</div>

<style>
  .hex-pane {
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 0.8rem;
    line-height: 1.4;
    white-space: pre;
    overflow: auto;
  }
  .hex-row {
    display: flex;
    gap: 1rem;
  }
  .hex-offset {
    color: #888;
  }
  .hex-cells {
    display: inline-flex;
    gap: 0.35rem;
  }
  .hex-ascii {
    color: #9ab;
  }
  .hex-empty {
    color: #888;
  }
</style>
