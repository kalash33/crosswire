<script lang="ts">
  // Key/value table for the Pretty tab (form bodies, query strings). Every
  // cell is captured data rendered by text interpolation only (§11.1); CR/LF
  // in a decoded value show as glyphs, never as line breaks.
  import { visibleLineBreaks } from '../http/headers';
  import type { KeyValue } from './pretty';

  let {
    rows,
    raw = null,
    label,
    testid,
  }: {
    rows: readonly KeyValue[];
    /** The undecoded string, shown underneath. */
    raw?: string | null;
    /** Accessible caption, e.g. "Form fields". */
    label: string;
    testid: string;
  } = $props();

  // A value longer than this is clipped by CSS; the full text sits in `title`.
  const LONG = 120;
</script>

<div class="kv" data-testid={testid}>
  <table>
    <caption>{label} <span class="count">{rows.length}</span></caption>
    <tbody>
      {#each rows as r, i (i)}
        <tr>
          <th scope="row" data-testid={`${testid}-key`}>{visibleLineBreaks(r.key)}</th>
          <td data-testid={`${testid}-value`} title={r.value.length > LONG ? r.value : undefined}
            ><span class="v">{visibleLineBreaks(r.value)}</span></td
          >
        </tr>
      {/each}
    </tbody>
  </table>
  {#if raw !== null}
    <pre class="raw" data-testid={`${testid}-raw`}>{raw}</pre>
  {/if}
</div>

<style>
  .kv {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
    min-width: 0;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    table-layout: fixed;
    font-family: var(--font-mono);
    font-size: var(--fs-sm);
  }
  caption {
    text-align: left;
    padding: 0 0 var(--s-1);
    font-family: var(--font-ui);
    font-size: var(--fs-sm);
    color: var(--fg-muted);
  }
  .count {
    color: var(--fg-faint);
    font-variant-numeric: tabular-nums;
  }
  tr {
    border-bottom: 1px solid var(--border-faint);
  }
  th,
  td {
    padding: 3px var(--s-2);
    vertical-align: top;
    text-align: left;
    overflow-wrap: anywhere;
  }
  th {
    width: 32%;
    font-weight: 500;
    color: var(--syn-key);
  }
  td {
    color: var(--fg);
  }
  .v {
    display: -webkit-box;
    -webkit-line-clamp: 4;
    line-clamp: 4;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .raw {
    margin: 0;
    padding: var(--s-2) var(--s-3);
    background: var(--bg-sunken);
    color: var(--fg-muted);
    font-family: var(--font-mono);
    font-size: var(--fs-sm);
    white-space: pre-wrap;
    word-break: break-all;
  }
</style>
