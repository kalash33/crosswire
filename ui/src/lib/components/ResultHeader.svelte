<script lang="ts">
  import type { SendOutcomeDto } from '../api/repeater';
  import { renderChanges } from '../editor/assisted';

  // The send result summary. EVERY value here is attacker-influenced (§11.1):
  // `scope_state`, the `skipped` reasons and the `redirects` URLs all originate
  // from the wire. They are rendered as TEXT ONLY via Svelte interpolation. In
  // particular the redirect URLs are plain text, NEVER <a href> links — a
  // dynamic href could carry a `javascript:`/attacker URL and would also trip
  // the §11.1 dynamic-URL lint.
  let { outcome }: { outcome: SendOutcomeDto } = $props();

  const changeChips = $derived(renderChanges(outcome.changes));
</script>

<div class="result-header" data-testid="result-header">
  <div class="timings">
    <span class="metric" data-testid="result-ttfb">TTFB {outcome.ttfb_ms} ms</span>
    <span class="metric" data-testid="result-total">Total {outcome.total_ms} ms</span>
    <span class="metric scope" data-testid="result-scope-state">scope: {outcome.scope_state}</span>
    {#if outcome.cert_override}
      <span class="badge cert" data-testid="result-cert-override" role="status"
        >upstream cert overridden</span
      >
    {/if}
  </div>

  {#if changeChips.length > 0}
    <div class="chips" data-testid="result-changes" aria-label="applied changes">
      {#each changeChips as c, i (i)}
        <span class="chip" data-testid="result-change"
          >{c.label}{#if c.state}: {c.state}{/if}</span
        >
      {/each}
    </div>
  {/if}

  {#if outcome.skipped.length > 0}
    <div class="chips" data-testid="result-skipped-list" aria-label="skipped">
      {#each outcome.skipped as reason, i (i)}
        <span class="chip skip" data-testid="result-skipped">skipped: {reason}</span>
      {/each}
    </div>
  {/if}

  {#if outcome.redirects.length > 0}
    <div class="redirects" data-testid="result-redirects">
      <span class="redirects-label">Redirects</span>
      <ol>
        <!-- TEXT ONLY: never <a href>. These URLs are attacker-influenced. -->
        {#each outcome.redirects as url, i (i)}
          <li data-testid="result-redirect">{url}</li>
        {/each}
      </ol>
    </div>
  {/if}
</div>

<style>
  .result-header {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    padding: 0.4rem 0.6rem;
    border: 1px solid #8884;
    border-radius: 4px;
    background: #1b1b24;
  }
  .timings {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.5rem;
  }
  .metric {
    font-size: 0.8rem;
    font-variant-numeric: tabular-nums;
  }
  .metric.scope {
    opacity: 0.85;
  }
  .badge.cert {
    font-size: 0.72rem;
    padding: 0.1rem 0.4rem;
    border-radius: 3px;
    background: #4a2b00;
    color: #ffd9a0;
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem;
  }
  .chip {
    font-size: 0.75rem;
    padding: 0.1rem 0.4rem;
    border-radius: 3px;
    background: #22333a;
    color: #bfe3ef;
  }
  .chip.skip {
    background: #333022;
    color: #efe3bf;
  }
  .redirects {
    font-size: 0.78rem;
  }
  .redirects-label {
    opacity: 0.75;
  }
  .redirects ol {
    margin: 0.2rem 0 0;
    padding-left: 1.2rem;
    word-break: break-all;
  }
</style>
