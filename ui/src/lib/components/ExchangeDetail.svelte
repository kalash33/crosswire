<script lang="ts">
  import CopyMenu from './CopyMenu.svelte';
  import TagsNotesEditor from './TagsNotesEditor.svelte';
  import { history } from '../stores/history.svelte';
  import ByteEditor from '../editor/ByteEditor.svelte';
  import { bytesToLatin1 } from '../editor/encoding';
  import { apiGet, apiGetBytes } from '../api/client';
  import { previewToken, previewSrc } from '../api/preview';
  import { ApiError } from '../api/errors';
  import type { ExchangeDto, HeadDto, MessageDto } from '../stores/history.svelte';

  // The selected exchange id. Changing it reloads the detail.
  let { id }: { id: string } = $props();

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

  function onTagsNotesSaved(ex: ExchangeDto): void {
    if (ex.id === id) exchange = ex;
    history.patchRow(ex.id, { tags: ex.tags ?? [], notes: ex.notes });
  }

  async function openPreview() {
    const current = id;
    previewError = null;
    try {
      const { path } = await previewToken(current);
      if (id === current) previewUrl = previewSrc(path);
    } catch (e) {
      if (id === current) previewError = e instanceof Error ? e.message : 'preview failed';
    }
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

  // A structured, text-only rendering of a non-raw head. Values are decoded
  // Latin-1 (text only — never interpreted as markup).
  function structuredHead(head: HeadDto): string {
    if (head.type === 'normalized') {
      return (head.map ?? [])
        .map(([name, valueB64]) => `${name}: ${bytesToLatin1(base64ToBytes(valueB64))}`)
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

  // A read-only fidelity/diagnostics summary for one message.
  function fidelityNote(msg: MessageDto): string {
    const parts = [`fidelity: ${msg.fidelity}`, `kind: ${msg.kind}`];
    const diags = msg.diagnostics ?? [];
    if (diags.length > 0) {
      parts.push(`${diags.length} diagnostic${diags.length === 1 ? '' : 's'}`);
    }
    return parts.join(' · ');
  }
</script>

<section class="detail" data-testid="exchange-detail">
  {#if loadError}
    <p class="error" data-testid="detail-error" role="alert">{loadError}</p>
  {:else if !exchange}
    <p data-testid="detail-loading">Loading exchange…</p>
  {:else}
    <header class="meta">
      <code class="id" data-testid="detail-id">{exchange.id}</code>
      <span class="chip">state: {exchange.state ?? 'unknown'}</span>
      <span class="chip">scope: {exchange.scope_state ?? 'unknown'}</span>
      <TagsNotesEditor
        id={exchange.id}
        tags={exchange.tags ?? []}
        notes={exchange.notes ?? ''}
        onSaved={onTagsNotesSaved}
      />
    </header>

    <!-- Request -->
    <div class="pane" data-testid="detail-request">
      <h3>Request</h3>
      <CopyMenu id={exchange.id} />
      <p class="note" data-testid="detail-req-fidelity">{fidelityNote(exchange.request)}</p>
      {#if exchange.request.diagnostics && exchange.request.diagnostics.length > 0}
        <ul class="diags" data-testid="detail-req-diagnostics">
          {#each exchange.request.diagnostics as d, i (i)}
            <li>{d.severity}: {d.code} @ {d.span.start}‥{d.span.end}</li>
          {/each}
        </ul>
      {/if}

      <h4>Head</h4>
      {#key exchange.id}
        {#if rawHeadBytes(exchange.request.head)}
          <div data-testid="detail-req-head" data-exact="true">
            <ByteEditor bytes={rawHeadBytes(exchange.request.head)!} readonly={true} />
          </div>
        {:else}
          <!-- Structured fallback: text only (no byte-exact head bytes). -->
          <pre class="structured" data-testid="detail-req-head" data-exact="false">{structuredHead(
              exchange.request.head,
            )}</pre>
        {/if}
      {/key}

      <h4>Body</h4>
      {#if reqBody === null}
        <p class="note">Loading body…</p>
      {:else if reqBody.error}
        <p class="error" role="alert">{reqBody.error}</p>
      {:else if reqBody.absent}
        <p class="note" data-testid="detail-req-body-empty">No request body.</p>
      {:else}
        {#key `${exchange.id}-req-${reqBody.bytes.length}`}
          <div data-testid="detail-req-body">
            <ByteEditor
              bytes={reqBody.bytes}
              readonly={true}
              diagnostic={truncationBanner(reqBody)}
            />
          </div>
        {/key}
      {/if}
    </div>

    <!-- Response -->
    <div class="pane" data-testid="detail-response">
      <h3>Response</h3>
      {#if !exchange.response}
        <p class="note" data-testid="detail-no-response">No response recorded.</p>
      {:else}
        <p class="note" data-testid="detail-resp-fidelity">{fidelityNote(exchange.response)}</p>
        {#if exchange.response.diagnostics && exchange.response.diagnostics.length > 0}
          <ul class="diags" data-testid="detail-resp-diagnostics">
            {#each exchange.response.diagnostics as d, i (i)}
              <li>{d.severity}: {d.code} @ {d.span.start}‥{d.span.end}</li>
            {/each}
          </ul>
        {/if}

        <h4>Head</h4>
        {#key exchange.id}
          {#if rawHeadBytes(exchange.response.head)}
            <div data-testid="detail-resp-head" data-exact="true">
              <ByteEditor bytes={rawHeadBytes(exchange.response.head)!} readonly={true} />
            </div>
          {:else}
            <pre class="structured" data-testid="detail-resp-head" data-exact="false">{structuredHead(
                exchange.response.head,
              )}</pre>
          {/if}
        {/key}

        <h4>Body</h4>
        {#if respBody === null}
          <p class="note">Loading body…</p>
        {:else if respBody.error}
          <p class="error" role="alert">{respBody.error}</p>
        {:else if respBody.absent}
          <p class="note" data-testid="detail-resp-body-empty">No response body.</p>
        {:else}
          {#key `${exchange.id}-resp-${respBody.bytes.length}`}
            <div data-testid="detail-resp-body">
              <ByteEditor
                bytes={respBody.bytes}
                readonly={true}
                diagnostic={truncationBanner(respBody)}
              />
            </div>
          {/key}
          <button type="button" data-testid="preview-button" onclick={openPreview}>Preview</button>
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
      {/if}
    </div>
  {/if}
</section>

<style>
  .detail {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    min-width: 0;
  }
  .meta {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.4rem;
  }
  .id {
    font-size: 0.8rem;
    opacity: 0.85;
  }
  .chip {
    font-size: 0.75rem;
    padding: 0.1rem 0.4rem;
    border-radius: 3px;
    background: #2a2a3a;
    color: #cfd0ff;
  }
  .pane {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    min-width: 0;
  }
  h3 {
    margin: 0.25rem 0 0;
    font-size: 0.95rem;
  }
  h4 {
    margin: 0.35rem 0 0;
    font-size: 0.8rem;
    opacity: 0.8;
  }
  .note {
    font-size: 0.8rem;
    opacity: 0.75;
    margin: 0;
  }
  .error {
    font-size: 0.85rem;
    color: #ff9b9b;
    margin: 0;
  }
  .structured {
    margin: 0;
    padding: 0.4rem 0.6rem;
    background: #1b1b24;
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 0.8rem;
    white-space: pre-wrap;
    word-break: break-all;
  }
  iframe {
    width: 100%;
    height: 20rem;
    border: 1px solid #333;
    background: #fff;
  }
  .diags {
    margin: 0;
    padding-left: 1rem;
    font-size: 0.78rem;
    color: #ffd9a0;
  }
</style>
