<script lang="ts">
  import ByteEditor from '../editor/ByteEditor.svelte';
  import HexPane from '../editor/HexPane.svelte';
  import CopyMenu from './CopyMenu.svelte';
  import HeadersTable from './HeadersTable.svelte';
  import ResultHeader from './ResultHeader.svelte';
  import { parseHeadLines, type HeaderLine } from '../http/headers';
  import { hexPage } from '../editor/hex-window';
  import { responseView, type RepeaterTab } from '../stores/repeater.svelte';
  import type { ExchangeDto } from '../stores/history.svelte';
  import { latin1ToBytes, bytesToLatin1 } from '../editor/encoding';
  import { apiGet, apiGetBytes } from '../api/client';
  import { ApiError } from '../api/errors';
  import { humanBytes } from '../format';
  import EmptyState from '../ui/EmptyState.svelte';
  import Kbd from '../ui/Kbd.svelte';
  import Tabs from '../ui/Tabs.svelte';
  import IconButton from '../ui/IconButton.svelte';

  // The response side of one repeater tab: empty state before the first send,
  // then a status line and Headers / Raw / Hex sub-tabs. Everything shown is
  // captured data and rendered as text only (§11.1).
  let { tab }: { tab: RepeaterTab } = $props();

  const view = $derived(responseView(tab));

  // ---- Response body -------------------------------------------------------
  // Sourced via apiGetBytes('/exchanges/{id}/body/resp') for the outcome's
  // exchange_id (the send response carries only the id, not the bytes). A 404
  // means the response had no body.
  interface RespState {
    bytes: Uint8Array;
    absent: boolean;
    error: string | null;
  }
  let resp = $state<RespState | null>(null);

  // ---- Response head (status line + headers) -------------------------------
  // From GET /exchanges/{id}. Best effort: a failure only empties the Headers
  // tab, it never hides the body.
  // `bytes` is set for a byte-exact HTTP/1 head (HeadersTable parses it and
  // renders the status line and any malformed lines itself); `structured` for
  // a normalized head, whose `:status` is shown here as the start line.
  interface HeadState {
    bytes: Uint8Array | null;
    structured: HeaderLine[] | null;
    start: string;
    count: number;
    note: string | null;
  }
  let head = $state<HeadState | null>(null);
  let pageNo = $state(0);

  function b64ToBytes(b64: string): Uint8Array {
    return latin1ToBytes(atob(b64));
  }

  function noteOnly(note: string): HeadState {
    return { bytes: null, structured: null, start: '', count: 0, note };
  }

  function headFrom(ex: ExchangeDto): HeadState {
    const h = ex.response?.head;
    if (!h) return noteOnly('No response head was recorded.');
    if (h.type === 'http1-raw') {
      const bytes = b64ToBytes(h.bytes);
      const p = parseHeadLines(bytes);
      return { bytes, structured: null, start: p.start, count: p.headers.length, note: null };
    }
    if (h.type === 'normalized') {
      const all = (h.map ?? []).map(([name, v]) => ({ name, value: bytesToLatin1(b64ToBytes(v)) }));
      const status = all.find((x) => x.name === ':status');
      const structured = all.filter((x) => x.name !== ':status');
      return {
        bytes: null,
        structured,
        start: status ? `:status ${status.value}` : '',
        count: structured.length,
        note: null,
      };
    }
    return noteOnly(`Opaque tunnel (${h.reason}).`);
  }

  // Status code from the start line ("HTTP/1.1 200 OK" or ":status 200").
  const status = $derived.by(() => {
    const m = /^(?:HTTP\/\S+|:status)\s+(\d{3})\b/.exec(head?.start ?? '');
    return m ? m[1] : null;
  });

  $effect(() => {
    const id = tab.outcome?.exchange_id;
    resp = null;
    head = null;
    pageNo = 0;
    if (!id) return;
    const current = id;
    const enc = encodeURIComponent(current);
    void (async () => {
      try {
        const { bytes } = await apiGetBytes(`/api/v1/exchanges/${enc}/body/resp`);
        if (tab.outcome?.exchange_id === current) resp = { bytes, absent: false, error: null };
      } catch (e) {
        if (tab.outcome?.exchange_id !== current) return;
        if (e instanceof ApiError && e.status === 404) {
          resp = { bytes: new Uint8Array(), absent: true, error: null };
        } else {
          resp = {
            bytes: new Uint8Array(),
            absent: false,
            error: e instanceof Error ? e.message : 'response body read failed',
          };
        }
      }
    })();
    void (async () => {
      try {
        const ex = await apiGet<ExchangeDto>(`/api/v1/exchanges/${enc}`);
        if (tab.outcome?.exchange_id === current) head = headFrom(ex);
      } catch (e) {
        if (tab.outcome?.exchange_id !== current) return;
        const msg = e instanceof Error ? e.message : 'request failed';
        head = noteOnly(`Headers unavailable: ${msg}`);
      }
    })();
  });

  // ---- Sub-tabs ----------------------------------------------------------------
  // Raw is the default: the body is what a send is usually checked for.
  type Sub = 'headers' | 'raw' | 'hex';
  let sub = $state<Sub>('raw');
  const subTabs = $derived([
    { id: 'headers', label: 'Headers', badge: head?.count, testid: 'resp-tab-headers' },
    { id: 'raw', label: 'Raw', testid: 'resp-tab-raw' },
    { id: 'hex', label: 'Hex', testid: 'resp-tab-hex' },
  ]);

  // ---- Hex paging: never more than one 64 KiB window in the DOM ----------------
  const win = $derived(hexPage(resp?.bytes.length ?? 0, pageNo));
  const hexBytes = $derived(resp ? resp.bytes.subarray(win.start, win.end) : new Uint8Array());
</script>

<div class="response" data-testid="repeater-response">
  {#if view === 'empty'}
    <EmptyState
      icon="send"
      title="Send the request to see the response"
      body="The status, timings, headers and body appear here."
      testid="response-empty"
    >
      {#snippet actions()}
        <span class="hint">Press <Kbd keys="Ctrl+Space" /> to send</span>
      {/snippet}
    </EmptyState>
  {:else if view === 'sending'}
    <EmptyState icon="send" title="Sending…" testid="response-sending" />
  {:else if view === 'error'}
    <div class="error-state" data-testid="response-error">
      <p class="error-title">Send failed</p>
      <p class="error-text" data-testid="tab-error" role="alert">{tab.error}</p>
      <p class="error-help">Check the target host, port and TLS settings, then send again.</p>
    </div>
  {:else if tab.outcome}
    <div class="head">
      <ResultHeader
        outcome={tab.outcome}
        {status}
        size={resp && !resp.error ? resp.bytes.length : null}
      />
      {#if tab.error}
        <p class="error-text" data-testid="tab-error" role="alert">Last send failed: {tab.error}</p>
      {/if}
      <CopyMenu id={tab.outcome.exchange_id} />
    </div>

    <div class="subtabs">
      <Tabs tabs={subTabs} active={sub} onselect={(id) => (sub = id as Sub)} label="Response view" />
      {#if tab.sending}<span class="resending">Sending…</span>{/if}
    </div>

    <div class="panel" role="tabpanel" aria-label={sub}>
      {#if sub === 'headers'}
        {#if head === null}
          <p class="note">Loading headers…</p>
        {:else}
          {#if head.note}
            <p class="note" data-testid="resp-headers-note">{head.note}</p>
          {:else if head.bytes}
            <HeadersTable bytes={head.bytes} testid="resp-headers" />
          {:else}
            {#if head.start}<p class="start-line" data-testid="resp-start-line">{head.start}</p>{/if}
            <HeadersTable structured={head.structured} testid="resp-headers" />
          {/if}
        {/if}
      {:else if resp === null}
        <p class="note">Loading response…</p>
      {:else if resp.error}
        <p class="error-text" role="alert" data-testid="resp-error">{resp.error}</p>
      {:else if resp.absent}
        <p class="note" data-testid="resp-empty">No response body.</p>
      {:else if sub === 'raw'}
        {#key `${tab.outcome.exchange_id}-${resp.bytes.length}`}
          <div class="raw" data-testid="result-resp-body">
            <ByteEditor bytes={resp.bytes} readonly={true} />
          </div>
        {/key}
      {:else}
        <div class="hex">
          {#if win.pages > 1}
            <div class="pager" data-testid="hex-pager">
              <IconButton
                icon="chevron-right"
                class="flip"
                size="sm"
                label="Previous 64 KiB"
                data-testid="hex-prev"
                disabled={win.page === 0}
                onclick={() => (pageNo = win.page - 1)}
              />
              <span class="range" data-testid="hex-range"
                >{`Bytes ${win.start}–${win.end - 1} of ${resp.bytes.length} (${humanBytes(resp.bytes.length)}), page ${win.page + 1} of ${win.pages}`}</span
              >
              <IconButton
                icon="chevron-right"
                size="sm"
                label="Next 64 KiB"
                data-testid="hex-next"
                disabled={win.page >= win.pages - 1}
                onclick={() => (pageNo = win.page + 1)}
              />
            </div>
          {/if}
          <HexPane bytes={hexBytes} base={win.start} />
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .response {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    min-width: 0;
  }
  .response > :global(.empty-state) {
    flex: 1;
  }
  .hint {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-sm);
  }
  .head {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
    padding: var(--s-2) var(--s-3);
    border-bottom: 1px solid var(--border);
  }
  /* The shared copy controls, compacted to the status-line scale. */
  .head :global(.copy-menu button) {
    height: var(--control-h-sm);
    padding: 0 var(--s-2);
    font-size: var(--fs-sm);
  }
  .subtabs {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    padding: 0 var(--s-3);
    border-bottom: 1px solid var(--border);
  }
  .resending {
    margin-left: auto;
    font-size: var(--fs-sm);
    color: var(--fg-muted);
  }
  .panel {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: var(--s-2) var(--s-3);
  }
  .start-line {
    margin: 0 0 var(--s-2);
    font-family: var(--font-mono);
    font-weight: 600;
    word-break: break-all;
  }
  .note {
    margin: 0;
    font-size: var(--fs-sm);
    color: var(--fg-muted);
  }
  .error-state {
    display: flex;
    flex-direction: column;
    gap: var(--s-1);
    padding: var(--s-6) var(--s-4);
    max-width: 60ch;
  }
  .error-title {
    margin: 0;
    font-size: var(--fs-lg);
    font-weight: 600;
    color: var(--danger);
  }
  .error-text {
    margin: 0;
    font-size: var(--fs-md);
    color: var(--danger);
    word-break: break-word;
  }
  .error-help {
    margin: 0;
    font-size: var(--fs-sm);
    color: var(--fg-muted);
  }
  .pager {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    margin-bottom: var(--s-2);
    font-size: var(--fs-sm);
    color: var(--fg-muted);
  }
  .pager :global(.flip svg) {
    transform: scaleX(-1);
  }
</style>
