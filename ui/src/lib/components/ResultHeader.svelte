<script lang="ts">
  import type { SendOutcomeDto } from '../api/repeater';
  import { renderChanges } from '../editor/assisted';
  import { humanBytes } from '../format';
  import Badge from '../ui/Badge.svelte';

  // The send result as a status line. EVERY value here is attacker-influenced
  // (§11.1): the status code, `scope_state`, the `skipped` reasons and the
  // `redirects` URLs all originate from the wire. They are rendered as TEXT
  // ONLY via Svelte interpolation. In particular the redirect URLs are plain
  // text, NEVER <a href> links: a dynamic href could carry a `javascript:` or
  // attacker URL and would also trip the §11.1 dynamic-URL lint.
  let {
    outcome,
    status = null,
    size = null,
  }: {
    outcome: SendOutcomeDto;
    /** Response status code parsed from the stored head, when known. */
    status?: string | null;
    /** Response body size in bytes, when loaded. */
    size?: number | null;
  } = $props();

  const changeChips = $derived(renderChanges(outcome.changes));
</script>

<div class="result-header" data-testid="result-header">
  <div class="line">
    {#if status}
      <span class="status" data-testid="result-status"><Badge kind="status" value={status} /></span>
    {/if}
    <span class="metric" data-testid="result-ttfb"><span class="k">TTFB</span> {outcome.ttfb_ms} ms</span>
    <span class="metric" data-testid="result-total"><span class="k">Total</span> {outcome.total_ms} ms</span>
    {#if size !== null}
      <span class="metric" data-testid="result-size"><span class="k">Size</span> {humanBytes(size)}</span>
    {/if}
    <span class="metric" data-testid="result-scope-state"
      ><span class="k">Scope</span> <Badge kind="scope" value={outcome.scope_state} /></span
    >
    {#if outcome.cert_override}
      <span class="badge cert" data-testid="result-cert-override" role="status"
        >Upstream cert overridden</span
      >
    {/if}
  </div>

  {#if changeChips.length > 0 || outcome.skipped.length > 0}
    <div class="chips">
      {#if changeChips.length > 0}
        <span class="chip-group" data-testid="result-changes" aria-label="applied changes">
          {#each changeChips as c, i (i)}
            <span class="chip" data-testid="result-change"
              >{c.label}{#if c.state}: {c.state}{/if}</span
            >
          {/each}
        </span>
      {/if}
      {#if outcome.skipped.length > 0}
        <span class="chip-group" data-testid="result-skipped-list" aria-label="skipped">
          {#each outcome.skipped as reason, i (i)}
            <span class="chip skip" data-testid="result-skipped">skipped: {reason}</span>
          {/each}
        </span>
      {/if}
    </div>
  {/if}

  {#if outcome.redirects.length > 0}
    <details class="redirects" data-testid="result-redirects" open>
      <summary>{outcome.redirects.length} redirect{outcome.redirects.length === 1 ? '' : 's'}</summary>
      <ol>
        <!-- TEXT ONLY: never <a href>. These URLs are attacker-influenced. -->
        {#each outcome.redirects as url, i (i)}
          <li data-testid="result-redirect">{url}</li>
        {/each}
      </ol>
    </details>
  {/if}
</div>

<style>
  .result-header {
    display: flex;
    flex-direction: column;
    gap: var(--s-1);
    min-width: 0;
  }
  .line {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-1) var(--s-4);
    min-width: 0;
  }
  .metric {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-sm);
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
    min-width: 0;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .k {
    font-family: var(--font-ui);
    color: var(--fg-muted);
  }
  .badge.cert {
    font-size: var(--fs-xs);
    padding: 1px 6px;
    border-radius: var(--r-1);
    background: var(--warn-bg);
    color: var(--warn-fg);
  }
  .chips,
  .chip-group {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-1);
  }
  .chips {
    gap: var(--s-1) var(--s-2);
  }
  .chip {
    font-size: var(--fs-xs);
    padding: 1px 6px;
    border-radius: var(--r-1);
    background: var(--info-bg);
    color: var(--info-fg);
  }
  .chip.skip {
    background: var(--warn-bg);
    color: var(--warn-fg);
  }
  .redirects {
    font-size: var(--fs-sm);
  }
  .redirects summary {
    cursor: pointer;
    color: var(--fg-muted);
  }
  .redirects ol {
    margin: var(--s-1) 0 0;
    padding-left: 1.4rem;
    font-family: var(--font-mono);
    word-break: break-all;
  }
</style>
