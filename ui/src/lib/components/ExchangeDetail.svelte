<script lang="ts" module>
  // The Headers/Raw/Hex choice per side outlives a row switch (the detail is
  // re-created for every exchange), so it lives at module level.
  type View = 'headers' | 'raw' | 'hex';
  // A narrow pane shows one message at a time; the response is the default.
  const views = $state<{ req: View; resp: View; side: 'req' | 'resp' }>({
    req: 'raw',
    resp: 'raw',
    side: 'resp',
  });
</script>

<script lang="ts">
  import { onMount } from 'svelte';
  import CopyDropdown from './CopyDropdown.svelte';
  import TagsNotesEditor from './TagsNotesEditor.svelte';
  import HeadersTable from './HeadersTable.svelte';
  import DiagnosticsList from './DiagnosticsList.svelte';
  import HexWindow from './HexWindow.svelte';
  import ByteEditor from '../editor/ByteEditor.svelte';
  import Badge from '../ui/Badge.svelte';
  import Button from '../ui/Button.svelte';
  import Tabs from '../ui/Tabs.svelte';
  import Segmented from '../ui/Segmented.svelte';
  import Splitter from '../ui/Splitter.svelte';
  import { toasts } from '../ui/toasts.svelte';
  import { history } from '../stores/history.svelte';
  import { decoder } from '../stores/decoder.svelte';
  import { router } from '../router.svelte';
  import { copyStatus } from '../copy-actions.svelte';
  import { selectionBytes } from '../decode/selection';
  import { bytesToLatin1 } from '../editor/encoding';
  import { parseHeadLines, type HeaderLine } from '../http/headers';
  import { humanBytes, timeShort } from '../format';
  import { apiGet, apiGetBytes } from '../api/client';
  import { previewToken, previewSrc } from '../api/preview';
  import { ApiError } from '../api/errors';
  import type { ExchangeDto, HeadDto, MessageDto, Row } from '../stores/history.svelte';

  // The selected exchange id (the parent re-creates this component per id) and
  // its history row, when loaded, for the title line.
  let { id, row = null }: { id: string; row?: Row | null } = $props();

  interface BodyState {
    bytes: Uint8Array;
    truncatedAt: number | null;
    absent: boolean;
    error: string | null;
  }

  let exchange = $state<ExchangeDto | null>(null);
  let loadError = $state<string | null>(null);
  let reqBody = $state<BodyState | null>(null);
  let respBody = $state<BodyState | null>(null);
  let previewUrl = $state<string | null>(null);
  let previewError = $state<string | null>(null);

  // The layout follows the DETAIL PANE's own width (bind:clientWidth uses a
  // ResizeObserver), not the window: from 760px the two messages sit side by
  // side, each the full height; below that a Request | Response switch shows
  // one at a time, also at full height.
  const SIDE_BY_SIDE_PX = 760;
  let paneWidth = $state(0);
  const wide = $derived(paneWidth >= SIDE_BY_SIDE_PX);

  onMount(() => copyStatus.clear());

  // The PATCH answers with an empty body, so merge the values that were sent.
  // A save that lands after a row switch still updates ITS row, never the
  // detail now showing another exchange.
  function onTagsNotesSaved(saved: { id: string; tags?: string[]; notes?: string }): void {
    const { id: savedId, ...values } = saved;
    if (exchange && exchange.id === savedId) exchange = { ...exchange, ...values };
    history.patchRow(savedId, values);
  }

  /** Render the response in the inert preview frame (also used by History's toolbar). */
  export async function openPreview(): Promise<void> {
    const current = id;
    previewError = null;
    views.side = 'resp';
    try {
      const { path } = await previewToken(current);
      if (id === current) previewUrl = previewSrc(path);
    } catch (e) {
      if (id === current) previewError = e instanceof Error ? e.message : 'preview failed';
    }
  }

  // Decode selection: read the selection at pointerdown, before the button
  // takes focus, then seed the Decoder and go there.
  let grabbed: { bytes: Uint8Array; nonLatin1: boolean } | null = null;
  function grabSelection(): void {
    grabbed = selectionBytes();
  }
  function decodeSelection(): void {
    const sel = grabbed ?? selectionBytes();
    grabbed = null;
    if (sel.bytes.length === 0) {
      toasts.push({ kind: 'info', text: 'Select bytes in the Raw view first, then Decode selection.' });
      return;
    }
    decoder.seed(sel.bytes, sel.nonLatin1);
    router.navigate('/decoder');
  }

  // Decode base64 (the model's bytes_serde wire form) into exact bytes.
  function base64ToBytes(b64: string): Uint8Array {
    const bin = atob(b64);
    const out = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
    return out;
  }

  // Raw head bytes when the head is HTTP/1 raw (byte-exact), else null.
  function rawHeadBytes(head: HeadDto): Uint8Array | null {
    return head.type === 'http1-raw' ? base64ToBytes(head.bytes) : null;
  }

  // Fields of a normalized head; values are Latin-1 text, never markup.
  function structuredFields(head: HeadDto): HeaderLine[] {
    if (head.type !== 'normalized') return [];
    return (head.map ?? []).map(([name, valueB64]) => ({
      name,
      value: bytesToLatin1(base64ToBytes(valueB64)),
    }));
  }

  // A structured, text-only rendering of a non-raw head.
  function structuredHead(head: HeadDto): string {
    if (head.type === 'normalized') {
      return structuredFields(head)
        .map((f) => `${f.name}: ${f.value}`)
        .join('\n');
    }
    if (head.type === 'tunnel') {
      return `Opaque tunnel (${head.reason}): ${head.bytes_in} bytes in, ${head.bytes_out} bytes out, ${head.duration_ms} ms`;
    }
    return '';
  }

  async function loadBody(side: 'req' | 'resp'): Promise<BodyState> {
    try {
      const { bytes, truncatedAt } = await apiGetBytes(`/api/v1/exchanges/${id}/body/${side}`);
      return { bytes, truncatedAt, absent: false, error: null };
    } catch (e) {
      // 404 = the message has no body (Body::None); anything else is a fault.
      if (e instanceof ApiError && e.status === 404) {
        return { bytes: new Uint8Array(), truncatedAt: null, absent: true, error: null };
      }
      return {
        bytes: new Uint8Array(),
        truncatedAt: null,
        absent: false,
        error: e instanceof Error ? e.message : 'body read failed',
      };
    }
  }

  // Reload whenever the selected id changes.
  $effect(() => {
    const current = id;
    exchange = null;
    loadError = null;
    reqBody = null;
    respBody = null;
    previewUrl = null;
    previewError = null;
    void (async () => {
      try {
        const ex = await apiGet<ExchangeDto>(`/api/v1/exchanges/${current}`);
        if (id !== current) return;
        exchange = ex;
      } catch (e) {
        if (id !== current) return;
        loadError = e instanceof Error ? e.message : 'failed to load exchange';
        return;
      }
      const req = await loadBody('req');
      if (id === current) reqBody = req;
      if (exchange?.response) {
        const resp = await loadBody('resp');
        if (id === current) respBody = resp;
      }
    })();
  });

  function truncationBanner(b: BodyState | null): string | null {
    if (!b || b.truncatedAt === null) return null;
    return `Partial capture: body truncated at ${b.truncatedAt} bytes. The bytes below are the captured prefix.`;
  }

  // A read-only fidelity summary for one message.
  function fidelityNote(msg: MessageDto): string {
    return `fidelity: ${msg.fidelity}, kind: ${msg.kind}`;
  }

  // Head + body bytes for the Hex tab (the head only when it is byte-exact).
  function hexBytes(msg: MessageDto, body: BodyState | null): Uint8Array {
    const head = rawHeadBytes(msg.head) ?? new Uint8Array();
    const b = body?.bytes ?? new Uint8Array();
    if (b.length === 0) return head;
    const out = new Uint8Array(head.length + b.length);
    out.set(head, 0);
    out.set(b, head.length);
    return out;
  }

  function headerCount(msg: MessageDto): number {
    const raw = rawHeadBytes(msg.head);
    return raw ? parseHeadLines(raw).headers.length : structuredFields(msg.head).length;
  }

  // Title line: prefer the history row; fall back to the request line.
  const reqStart = $derived.by(() => {
    if (!exchange) return null;
    const raw = rawHeadBytes(exchange.request.head);
    return raw ? parseHeadLines(raw).start : null;
  });
  const titleMethod = $derived(row?.method ?? reqStart?.split(' ')[0] ?? '');
  const titlePath = $derived(row?.path ?? reqStart?.split(' ')[1] ?? '');
  const fullTime = $derived.by(() => {
    if (!row) return '';
    try {
      return new Date(row.created_at).toLocaleString();
    } catch {
      return String(row.created_at);
    }
  });

  function tabsFor(msg: MessageDto) {
    const d = msg.diagnostics?.length ?? 0;
    return [
      { id: 'headers', label: 'Headers', badge: headerCount(msg), testid: 'detail-tab-headers' },
      { id: 'raw', label: 'Raw', badge: d > 0 ? `${d} diag` : undefined, testid: 'detail-tab-raw' },
      { id: 'hex', label: 'Hex', testid: 'detail-tab-hex' },
    ];
  }
</script>

{#snippet bodyBlock(side: 'req' | 'resp', body: BodyState | null)}
  {#if body === null}
    <p class="note">Loading body…</p>
  {:else if body.error}
    <p class="error" role="alert">{body.error}</p>
  {:else if body.absent}
    <p class="note" data-testid={`detail-${side}-body-empty`}>
      No {side === 'req' ? 'request' : 'response'} body.
    </p>
  {:else}
    {#key `${id}-${side}-${body.bytes.length}`}
      <div data-testid={`detail-${side}-body`}>
        <ByteEditor bytes={body.bytes} readonly={true} diagnostic={truncationBanner(body)} />
      </div>
    {/key}
  {/if}
{/snippet}

{#snippet message(side: 'req' | 'resp', msg: MessageDto, body: BodyState | null)}
  {@const view = views[side]}
  <div class="toolbar">
    <span class="ptitle">{side === 'req' ? 'Request' : 'Response'}</span>
    <Tabs
      tabs={tabsFor(msg)}
      active={view}
      label={side === 'req' ? 'Request view' : 'Response view'}
      onselect={(v) => (views[side] = v as View)}
    />
    <span class="spacer"></span>
    {#if side === 'resp' && body && !body.absent && !body.error}
      <Button size="sm" icon="external" data-testid="preview-button" onclick={openPreview}>Preview</Button>
    {/if}
  </div>
  <p class="note" data-testid={`detail-${side}-fidelity`}>{fidelityNote(msg)}</p>
  {#if msg.diagnostics && msg.diagnostics.length > 0}
    <DiagnosticsList diagnostics={msg.diagnostics} testid={`detail-${side}-diagnostics`} />
  {/if}

  {#if side === 'resp'}
    {#if previewError}
      <p class="error" role="alert" data-testid="preview-error">{previewError}</p>
    {/if}
    {#if previewUrl}
      <!-- Inert render on the 127.0.0.1 origin: no allow-scripts, no allow-same-origin. -->
      <!-- §11.1 allowlist: previewSrc() hard-codes the http://127.0.0.1:<port> origin and only accepts a /preview/ path. -->
      <!-- eslint-disable no-restricted-syntax -->
      <iframe
        data-testid="preview-frame"
        title="Response preview"
        sandbox=""
        referrerpolicy="no-referrer"
        src={previewUrl}
      ></iframe>
      <!-- eslint-enable no-restricted-syntax -->
    {/if}
  {/if}

  <div class="panel" role="tabpanel" data-testid={`detail-${side}-panel-${view}`}>
    {#if view === 'headers'}
      {#if rawHeadBytes(msg.head)}
        <HeadersTable bytes={rawHeadBytes(msg.head)} testid={`detail-${side}-headers`} />
      {:else if msg.head.type === 'normalized'}
        <HeadersTable structured={structuredFields(msg.head)} testid={`detail-${side}-headers`} />
      {:else}
        <p class="note">{structuredHead(msg.head)}</p>
      {/if}
    {:else if view === 'raw'}
      {#key id}
        {#if rawHeadBytes(msg.head)}
          <div data-testid={`detail-${side}-head`} data-exact="true">
            <ByteEditor bytes={rawHeadBytes(msg.head)!} readonly={true} />
          </div>
        {:else}
          <!-- Structured fallback: text only (no byte-exact head bytes). -->
          <pre class="structured" data-testid={`detail-${side}-head`} data-exact="false">{structuredHead(
              msg.head,
            )}</pre>
        {/if}
      {/key}
      {@render bodyBlock(side, body)}
    {:else}
      {#key `${id}-${side}-hex-${body?.bytes.length ?? -1}`}
        <HexWindow bytes={hexBytes(msg, body)} testid={`detail-${side}-hex`} />
      {/key}
      {#if rawHeadBytes(msg.head) === null}
        <p class="note">Body only: this head was not recorded byte-exact.</p>
      {/if}
    {/if}
  </div>
{/snippet}

{#snippet requestPane()}
  {#if exchange}
    <div class="pane" data-testid="detail-request">
      {@render message('req', exchange.request, reqBody)}
    </div>
  {/if}
{/snippet}

{#snippet responsePane()}
  {#if exchange}
    <div class="pane" data-testid="detail-response">
      {#if !exchange.response}
        <div class="toolbar"><span class="ptitle">Response</span></div>
        <p class="note" data-testid="detail-no-response">No response recorded.</p>
      {:else}
        {@render message('resp', exchange.response, respBody)}
      {/if}
    </div>
  {/if}
{/snippet}

<section class="detail" data-testid="exchange-detail" bind:clientWidth={paneWidth}>
  {#if loadError}
    <p class="error" data-testid="detail-error" role="alert">{loadError}</p>
  {:else if !exchange}
    <p class="note pad" data-testid="detail-loading">Loading exchange…</p>
  {:else}
    <header class="head">
      <div class="titleline">
        <h2 class="title" title={`${titleMethod} ${titlePath}`}>
          <span class="method">{titleMethod}</span>
          <span class="path">{titlePath}</span>
        </h2>
        <CopyDropdown
          {id}
          row={row ? { host: row.host, path: row.path } : null}
          chords={true}
          showStatus={true}
        />
        <Button
          size="sm"
          variant="ghost"
          icon="decoder"
          kbd="Ctrl+Shift+D"
          data-testid="decode-selection"
          onpointerdown={grabSelection}
          onclick={decodeSelection}>Decode selection</Button
        >
      </div>
      <div class="meta">
        {#if row?.host}<span class="host">{row.host}</span>{/if}
        {#if row?.status !== null && row?.status !== undefined}<Badge kind="status" value={row.status} />{/if}
        <Badge kind="state" value={exchange.state ?? 'unknown'} />
        <span class="scope"><span class="dim">scope</span><Badge kind="scope" value={exchange.scope_state ?? 'unknown'} /></span>
        {#if row?.resp_len !== null && row?.resp_len !== undefined}
          <span class="dim">{humanBytes(row.resp_len)}</span>
        {/if}
        {#if row}<span class="dim" title={fullTime}>{timeShort(row.created_at)}</span>{/if}
        <code class="id" data-testid="detail-id">{exchange.id}</code>
      </div>
      <TagsNotesEditor
        id={exchange.id}
        tags={exchange.tags ?? []}
        notes={exchange.notes ?? ''}
        onSaved={onTagsNotesSaved}
      />
    </header>

    <div class="sides">
      {#if wide}
        <Splitter direction="row" storageKey="history-messages" min={[300, 300]} label="Resize request and response">
          {#snippet a()}{@render requestPane()}{/snippet}
          {#snippet b()}{@render responsePane()}{/snippet}
        </Splitter>
      {:else}
        <div class="switch">
          <Segmented
            size="sm"
            label="Message"
            value={views.side}
            onchange={(v) => (views.side = v)}
            options={[
              { value: 'req', label: 'Request', testid: 'detail-side-req' },
              { value: 'resp', label: 'Response', testid: 'detail-side-resp' },
            ]}
          />
        </div>
        <div class="single">
          {#if views.side === 'req'}{@render requestPane()}{:else}{@render responsePane()}{/if}
        </div>
      {/if}
    </div>
  {/if}
</section>

<style>
  .detail {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-width: 0;
    min-height: 0;
  }
  .head {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
    padding: var(--s-3);
    border-bottom: 1px solid var(--border);
  }
  .titleline {
    display: flex;
    align-items: flex-start;
    gap: var(--s-2);
    min-width: 0;
  }
  .title {
    flex: 1 1 auto;
    min-width: 0;
    margin: 0;
    font-family: var(--font-mono);
    font-size: var(--fs-xl);
    font-weight: 500;
    line-height: 1.35;
    color: var(--fg);
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    overflow-wrap: anywhere;
  }
  .method {
    font-weight: 700;
    color: var(--accent);
    margin-right: var(--s-2);
  }
  .meta {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-1) var(--s-2);
    font-size: var(--fs-sm);
  }
  .host {
    font-family: var(--font-mono);
    color: var(--fg-muted);
    overflow-wrap: anywhere;
  }
  .scope {
    display: inline-flex;
    align-items: center;
    gap: var(--s-1);
  }
  .dim {
    color: var(--fg-faint);
    font-variant-numeric: tabular-nums;
  }
  .id {
    margin-left: auto;
    font-size: var(--fs-xs);
    color: var(--fg-faint);
  }
  .sides {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .switch {
    padding: var(--s-2) var(--s-3) 0;
  }
  .single {
    flex: 1 1 auto;
    min-height: 0;
    overflow: auto;
  }
  .pane {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
    min-width: 0;
    padding: var(--s-2) var(--s-3) var(--s-3);
  }
  .toolbar {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--s-2);
    min-width: 0;
  }
  .ptitle {
    font-size: var(--fs-md);
    font-weight: 600;
    color: var(--fg);
    margin-right: var(--s-1);
  }
  .spacer {
    flex: 1 1 auto;
  }
  .panel {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
    min-width: 0;
  }
  .note {
    font-size: var(--fs-sm);
    color: var(--fg-faint);
    margin: 0;
  }
  .pad {
    padding: var(--s-3);
  }
  .error {
    font-size: var(--fs-md);
    color: var(--danger);
    margin: 0;
    padding: var(--s-2) var(--s-3);
  }
  .structured {
    margin: 0;
    padding: var(--s-2) var(--s-3);
    background: var(--bg-sunken);
    font-family: var(--font-mono);
    font-size: var(--fs-sm);
    white-space: pre-wrap;
    word-break: break-all;
  }
  iframe {
    width: 100%;
    height: 20rem;
    border: 1px solid var(--border);
    border-radius: var(--r-1);
    background: var(--preview-bg);
  }
</style>
