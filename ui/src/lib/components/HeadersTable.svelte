<script lang="ts">
  // Name/value view of a recorded head (Headers tab). Every cell is captured
  // data rendered by text interpolation only (§11.1). Raw heads are split by
  // parseHeadLines, which never yields CR or LF; structured (normalized) heads
  // pass through visibleLineBreaks so a CR/LF byte in a value shows as a glyph.
  import { parseHeadLines, visibleLineBreaks, type HeaderLine } from '../http/headers';

  let {
    bytes = null,
    structured = null,
    testid = 'headers-table',
  }: {
    /** Raw HTTP/1 head bytes (byte-exact). */
    bytes?: Uint8Array | null;
    /** Already-split fields (normalized heads), used when `bytes` is null. */
    structured?: HeaderLine[] | null;
    testid?: string;
  } = $props();

  const view = $derived.by(() => {
    if (bytes) return parseHeadLines(bytes);
    return {
      start: '',
      headers: (structured ?? []).map((h) => ({
        name: visibleLineBreaks(h.name),
        value: visibleLineBreaks(h.value),
      })),
      malformed: [] as string[],
    };
  });
</script>

<div class="headers" data-testid={testid}>
  {#if view.start}
    <div class="start" data-testid="headers-start">{view.start}</div>
  {/if}
  {#if view.headers.length === 0}
    <p class="none">No header fields.</p>
  {:else}
    <table>
      <tbody>
        {#each view.headers as h, i (i)}
          <tr>
            <th scope="row">{h.name}</th>
            <td>{h.value}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
  {#if view.malformed.length > 0}
    <div class="malformed" data-testid="headers-malformed">
      <p class="label">{view.malformed.length} line{view.malformed.length === 1 ? '' : 's'} not in name: value form</p>
      <ul>
        {#each view.malformed as line, i (i)}<li>{line}</li>{/each}
      </ul>
    </div>
  {/if}
</div>

<style>
  .headers {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
    min-width: 0;
    font-family: var(--font-mono);
    font-size: var(--fs-sm);
  }
  .start {
    padding: var(--s-1) var(--s-2);
    background: var(--bg-sunken);
    border-radius: var(--r-1);
    color: var(--fg);
    overflow-wrap: anywhere;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    table-layout: fixed;
  }
  th,
  td {
    padding: 3px var(--s-2);
    border-bottom: 1px solid var(--border-faint);
    vertical-align: top;
    text-align: left;
    overflow-wrap: anywhere;
    white-space: pre-wrap;
  }
  th {
    width: 32%;
    font-weight: 500;
    color: var(--fg-muted);
  }
  td {
    color: var(--fg);
  }
  tr:hover th,
  tr:hover td {
    background: var(--bg-hover);
  }
  .none,
  .label {
    margin: 0;
    font-family: var(--font-ui);
    color: var(--fg-faint);
  }
  .malformed {
    padding: var(--s-2);
    border-left: 2px solid var(--warn);
    background: var(--warn-bg);
    color: var(--warn-fg);
  }
  .malformed .label {
    color: inherit;
  }
  .malformed ul {
    margin: var(--s-1) 0 0;
    padding-left: var(--s-4);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
</style>
