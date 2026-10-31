<script lang="ts">
  // Parse diagnostics grouped by code: one collapsible row per code with a
  // count, the spans inside. Codes are captured-path data: text only, and the
  // severity class comes from a closed set (never built from the raw value).
  import Badge from '../ui/Badge.svelte';
  import type { DiagnosticDto } from '../stores/history.svelte';

  let { diagnostics, testid }: { diagnostics: readonly DiagnosticDto[]; testid?: string } = $props();

  type Sev = 'error' | 'warn' | 'info';
  const RANK: Record<Sev, number> = { info: 0, warn: 1, error: 2 };

  function sev(s: string): Sev {
    const v = s.toLowerCase();
    if (v === 'error' || v === 'fatal') return 'error';
    if (v === 'warn' || v === 'warning') return 'warn';
    return 'info';
  }

  const groups = $derived.by(() => {
    const out: { code: string; sev: Sev; spans: DiagnosticDto['span'][] }[] = [];
    for (const d of diagnostics) {
      let g = out.find((x) => x.code === d.code);
      if (!g) {
        g = { code: d.code, sev: 'info', spans: [] };
        out.push(g);
      }
      const s = sev(d.severity);
      if (RANK[s] > RANK[g.sev]) g.sev = s;
      g.spans.push(d.span);
    }
    return out;
  });
</script>

<div class="diags" data-testid={testid}>
  {#each groups as g (g.code)}
    <details class="group sev-{g.sev}" data-testid="diag-group" data-code={g.code} data-count={g.spans.length}>
      <summary>
        <span class="mark" aria-hidden="true"></span>
        <Badge kind="neutral" value={`${g.spans.length}×`} />
        <span class="code">{g.code}</span>
        <span class="sev">{g.sev}</span>
      </summary>
      <ul>
        {#each g.spans as sp, i (i)}
          <li>bytes {sp.start}–{sp.end}</li>
        {/each}
      </ul>
    </details>
  {/each}
</div>

<style>
  .diags {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .group {
    border-radius: var(--r-1);
    background: var(--bg-sunken);
  }
  summary {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    padding: 3px var(--s-2);
    cursor: pointer;
    font-size: var(--fs-sm);
    list-style: none;
  }
  summary::-webkit-details-marker {
    display: none;
  }
  .mark {
    width: 6px;
    height: 6px;
    border-radius: var(--r-pill);
    flex: none;
    background: var(--info);
  }
  .sev-warn .mark {
    background: var(--warn);
  }
  .sev-error .mark {
    background: var(--danger);
  }
  .code {
    font-family: var(--font-mono);
    color: var(--fg);
    overflow-wrap: anywhere;
    min-width: 0;
  }
  .sev {
    margin-left: auto;
    font-size: var(--fs-xs);
    color: var(--fg-faint);
  }
  .sev-warn .sev {
    color: var(--warn);
  }
  .sev-error .sev {
    color: var(--danger);
  }
  ul {
    margin: 0;
    padding: 0 var(--s-2) var(--s-1) calc(var(--s-4) + var(--s-2));
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    color: var(--fg-muted);
  }
</style>
