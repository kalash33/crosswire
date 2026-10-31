<script lang="ts">
  // Header name/value table. Names and values are captured data: rendered as
  // text interpolation only (§11.1).
  //
  // NOTE: minimal local version so the Repeater builds on its own; Task 2 owns
  // this component and its version replaces this one on merge.
  import type { HeaderLine } from '../http/headers';

  let { headers }: { headers: HeaderLine[] } = $props();
</script>

<table class="headers-table" data-testid="headers-table">
  <tbody>
    {#each headers as h, i (i)}
      <tr>
        <th scope="row">{h.name}</th>
        <td>{h.value}</td>
      </tr>
    {/each}
  </tbody>
</table>

<style>
  .headers-table {
    width: 100%;
    border-collapse: collapse;
    font-family: var(--font-mono);
    font-size: var(--fs-sm);
  }
  th,
  td {
    padding: var(--s-1) var(--s-2);
    border-bottom: 1px solid var(--border);
    text-align: left;
    vertical-align: top;
    word-break: break-all;
  }
  th {
    width: 1%;
    white-space: nowrap;
    font-weight: 500;
    color: var(--fg-muted);
  }
</style>
