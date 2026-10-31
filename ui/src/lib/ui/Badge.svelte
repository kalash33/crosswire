<script lang="ts">
  // Compact coloured label. Values are captured data: text only, and the class
  // is chosen from a closed set (never built from the raw value).
  type Kind = 'method' | 'status' | 'state' | 'scope' | 'neutral';

  let { kind = 'neutral', value }: { kind?: Kind; value: string | number | null | undefined } = $props();

  const METHODS = new Set(['GET', 'POST', 'PUT', 'DELETE', 'PATCH']);

  const tone = $derived.by(() => {
    const v = value === null || value === undefined ? '' : String(value);
    switch (kind) {
      case 'method': {
        const m = v.toUpperCase();
        return METHODS.has(m) ? `m-${m.toLowerCase()}` : 'm-other';
      }
      case 'status': {
        const c = /^[1-5]\d\d$/.test(v) ? v[0] : '';
        return c ? `s-${c}xx` : 'neutral';
      }
      case 'state':
        return v === 'committed' ? 'ok' : v === 'error' ? 'danger' : v === '' ? 'neutral' : 'warn';
      case 'scope':
        return v === 'in' ? 'ok' : v === 'out' ? 'faint' : 'neutral';
      default:
        return 'neutral';
    }
  });
</script>

<span class="badge {kind} {tone}">{value ?? '—'}</span>

<style>
  .badge {
    display: inline-flex;
    align-items: center;
    height: 18px;
    padding: 0 6px;
    font-size: var(--fs-xs);
    font-weight: 600;
    line-height: 1;
    white-space: nowrap;
    color: var(--fg-muted);
    background: var(--chip-bg);
    border-radius: var(--r-1);
    font-variant-numeric: tabular-nums;
  }
  .badge.method,
  .badge.status {
    font-family: var(--font-mono);
    background: transparent;
    box-shadow: inset 0 0 0 1px currentColor;
  }
  .m-get {
    color: var(--method-get);
  }
  .m-post {
    color: var(--method-post);
  }
  .m-put {
    color: var(--method-put);
  }
  .m-delete {
    color: var(--method-delete);
  }
  .m-patch {
    color: var(--method-patch);
  }
  .m-other {
    color: var(--method-other);
  }
  .s-1xx {
    color: var(--status-1xx);
  }
  .s-2xx {
    color: var(--status-2xx);
  }
  .s-3xx {
    color: var(--status-3xx);
  }
  .s-4xx {
    color: var(--status-4xx);
  }
  .s-5xx {
    color: var(--status-5xx);
  }
  .badge.ok {
    color: var(--ok-fg);
    background: var(--ok-bg);
  }
  .badge.warn {
    color: var(--warn-fg);
    background: var(--warn-bg);
  }
  .badge.danger {
    color: var(--danger-fg);
    background: var(--danger-bg);
  }
  .badge.faint {
    color: var(--fg-faint);
  }
</style>
