<script lang="ts" module>
  /** Pretty and Raw render at most this much of a body until "Show all". */
  export const RENDER_LIMIT = 1024 * 1024;
</script>

<script lang="ts">
  // One HTTP message (U2 D1): Pretty | Raw | Headers | Hex, with wrap, control
  // characters and find. Used by the History detail (both sides) and the
  // Repeater response. Everything shown is captured data rendered as text only
  // (§11.1): editors hold the bytes as a Latin-1 document, tables interpolate
  // text, the Pretty views are derived in the UI and never sent anywhere.
  //
  // Test ids: the tab strip and toolbar use `testidPrefix` (`${p}-tab-raw`,
  // `${p}-wrap`, ...) so the History detail keeps `detail-tab-*` and the
  // Repeater keeps `resp-tab-*`; the content ids each host already had come in
  // through `ids`.
  import { onMount, type Snippet } from 'svelte';
  import ByteEditor from '../editor/ByteEditor.svelte';
  import HeadersTable from '../components/HeadersTable.svelte';
  import HexWindow from '../components/HexWindow.svelte';
  import IconButton from '../ui/IconButton.svelte';
  import Button from '../ui/Button.svelte';
  import Tabs from '../ui/Tabs.svelte';
  import PrettyJson from './PrettyJson.svelte';
  import KvTable from './KvTable.svelte';
  import FindBar from './FindBar.svelte';
  import { bytesToLatin1 } from '../editor/encoding';
  import { parseHeadLines } from '../http/headers';
  import { humanBytes } from '../format';
  import { findHint } from '../keymap-hints.svelte';
  import { keymap } from '../keymap';
  import type { EditorLanguage } from '../editor/lang';
  import {
    contentTypeOf,
    defaultTab as pickDefault,
    detectPretty,
    isJsonType,
    isMarkupType,
    mediaType,
    PRETTY_LIMIT,
    type ViewerTab,
  } from './detect';
  import { countPairs, isValidUtf8, parseForm, parseQuery, prettyJson } from './pretty';
  import { FIND_LIMIT, findAll, foldCase, nextMatch, prevMatch, type EditorMatches } from './find';

  interface Ids {
    /** Wrapper of the body editor in Raw. */
    body: string;
    /** The byte-exact head editor in Raw (data-exact="true"). */
    head?: string;
    headers: string;
    hex: string;
    /** Base of the Pretty views: `${pretty}-json`, `-form`, `-query`, `-note`. */
    pretty: string;
    /** When set, the panel carries `${panel}-panel-${tab}`. */
    panel?: string;
  }

  let {
    head,
    body,
    contentType = undefined,
    target = undefined,
    side,
    testidPrefix,
    ids,
    defaultTab = undefined,
    label,
    bodyDiagnostic = null,
    headerCount = undefined,
    rawBadge = undefined,
    title,
    actions,
    above,
    bodyStatus,
    headFallback,
    headersFallback,
  }: {
    /** Byte-exact HTTP/1 head, or null when the head was not recorded raw. */
    head: Uint8Array | null;
    /** The body bytes; null while the host shows `bodyStatus` (loading, absent, error). */
    body: Uint8Array | null;
    /** Media type when the head is not raw (normalized heads). */
    contentType?: string;
    /** The request target (request side), for the query table. */
    target?: string;
    side: 'req' | 'resp';
    testidPrefix: string;
    ids: Ids;
    defaultTab?: ViewerTab;
    /** Accessible name of the tab list. */
    label: string;
    bodyDiagnostic?: string | null;
    headerCount?: number;
    rawBadge?: string;
    title?: Snippet;
    actions?: Snippet;
    above?: Snippet;
    bodyStatus?: Snippet;
    headFallback?: Snippet;
    headersFallback?: Snippet;
  } = $props();

  const EMPTY = new Uint8Array();
  const bytes = $derived(body ?? EMPTY);
  const type = $derived(contentType ? mediaType(contentType) : contentTypeOf(head));
  const query = $derived(side === 'req' && target !== undefined && target.includes('?') ? target : undefined);
  const detection = $derived(detectPretty(head, bytes, type, query));
  const queryRows = $derived(query !== undefined ? parseQuery(query) : []);
  const prettyText = $derived(
    detection.kind === 'json' && !detection.tooLarge && body !== null ? prettyJson(bytes) : null,
  );
  // The table renders at most KV_LIMIT rows, so decode only one past that and
  // count the rest without decoding (a 2 MiB body of `a=&` is ~700k pairs).
  const KV_LIMIT = 1000;
  const isForm = $derived(detection.kind === 'form' && !detection.tooLarge && body !== null);
  const formRows = $derived(isForm ? parseForm(bytes, KV_LIMIT + 1) : []);
  const formTotal = $derived(isForm ? countPairs(bytes) : 0);
  // The raw string under the form table is a reference (KvTable cuts it at 64 KiB).
  const formRaw = $derived(isForm ? bytesToLatin1(bytes.subarray(0, 65 * 1024)) : '');
  // A Pretty body that is not strict UTF-8 shows U+FFFD; say so.
  const lossy = $derived(
    (prettyText !== null || isForm) && body !== null ? !isValidUtf8(bytes) : false,
  );

  // The tab follows the content until the user picks one.
  let chosen = $state<ViewerTab | null>(null);
  const auto = $derived.by((): ViewerTab => {
    if (defaultTab) return defaultTab;
    // Typed JSON that does not parse has nothing pretty to show.
    if (detection.kind === 'json' && !detection.tooLarge && body !== null && prettyText === null) {
      return queryRows.length > 0 ? 'pretty' : 'raw';
    }
    return pickDefault(detection, type);
  });
  const tab = $derived(chosen ?? auto);

  const bodyLanguage = $derived.by((): EditorLanguage | null => {
    if (isJsonType(type) || (detection.kind === 'json' && detection.reason === 'parse')) return 'json';
    if (isMarkupType(type)) return 'markup';
    return null;
  });

  // ---- Toggles ---------------------------------------------------------------
  let wrap = $state(true);
  let showControls = $state(false);
  let showAll = $state(false);
  const textual = $derived(tab === 'pretty' || tab === 'raw');

  const shown = $derived(showAll || bytes.length <= RENDER_LIMIT ? bytes : bytes.subarray(0, RENDER_LIMIT));
  const clipped = $derived(shown.length < bytes.length);

  // ---- Find --------------------------------------------------------------------
  let findOpen = $state(false);
  let findQuery = $state('');
  let caseSensitive = $state(false);
  let current = $state(-1);
  let findBar = $state<{ focus: () => void }>();
  let root = $state<HTMLElement>();

  // The documents find searches, in display order. They do not depend on the
  // query, so their case-folded copies are built once per document, not on
  // every keystroke.
  const docs = $derived.by((): { key: 'head' | 'body' | 'pretty'; text: string }[] => {
    if (!findOpen) return [];
    if (tab === 'raw') {
      const out: { key: 'head' | 'body' | 'pretty'; text: string }[] = [];
      if (head) out.push({ key: 'head', text: bytesToLatin1(head) });
      if (body !== null) out.push({ key: 'body', text: bytesToLatin1(shown) });
      return out;
    }
    if (tab === 'pretty' && prettyText !== null) return [{ key: 'pretty', text: prettyText }];
    return [];
  });
  const folded = $derived(caseSensitive ? [] : docs.map((d) => foldCase(d.text)));
  // At most FIND_LIMIT matches over all documents; one more means "10000+".
  const search = $derived.by(() => {
    const out: { key: 'head' | 'body' | 'pretty'; positions: number[] }[] = [];
    let left = FIND_LIMIT;
    let capped = false;
    docs.forEach((d, i) => {
      if (findQuery === '') return;
      // findAll itself stops one past FIND_LIMIT, so each call is bounded.
      const hits = findAll(d.text, findQuery, { caseSensitive, folded: folded[i] });
      if (hits.length > left) capped = true;
      const kept = hits.slice(0, left);
      left -= kept.length;
      out.push({ key: d.key, positions: kept });
    });
    return { found: out, capped };
  });
  const found = $derived(search.found);
  const total = $derived(found.reduce((n, f) => n + f.positions.length, 0));

  // A new search starts at the first match.
  $effect(() => {
    void findQuery;
    void caseSensitive;
    void tab;
    current = total > 0 ? 0 : -1;
  });

  function matchesFor(key: 'head' | 'body' | 'pretty'): EditorMatches | null {
    let offset = 0;
    for (const f of found) {
      if (f.key === key) {
        const local = current - offset;
        return {
          positions: f.positions,
          length: findQuery.length,
          current: local >= 0 && local < f.positions.length ? local : -1,
        };
      }
      offset += f.positions.length;
    }
    return null;
  }

  function openFind(): void {
    if (findOpen) findBar?.focus();
    findOpen = true;
  }
  function closeFind(): void {
    findOpen = false;
    root?.focus();
  }

  // Find is a keymap action in the `viewer` scope, bound to this root: while
  // focus is inside the viewer its chord (Ctrl+F by default) wins over a view
  // or global action on the same chord. On a textual tab it opens find;
  // elsewhere (Headers, Hex) it declines, so the browser's own find runs and
  // no outer scope sees the key.
  onMount(() => {
    const el = root;
    if (!el) return;
    return keymap.register(
      'find',
      () => {
        if (!textual) return false;
        openFind();
      },
      { el },
    );
  });

  // ---- Tabs ----------------------------------------------------------------------
  const count = $derived(headerCount ?? (head ? parseHeadLines(head).headers.length : undefined));
  const prettyBadge = $derived(
    detection.kind === 'none' && queryRows.length === 0
      ? undefined
      : detection.kind === 'json'
        ? 'JSON'
        : detection.kind === 'form'
          ? 'form'
          : 'query',
  );
  const tabs = $derived([
    { id: 'pretty', label: 'Pretty', badge: prettyBadge, testid: `${testidPrefix}-tab-pretty` },
    { id: 'raw', label: 'Raw', badge: rawBadge, testid: `${testidPrefix}-tab-raw` },
    { id: 'headers', label: 'Headers', badge: count, testid: `${testidPrefix}-tab-headers` },
    { id: 'hex', label: 'Hex', testid: `${testidPrefix}-tab-hex` },
  ]);

  function hexBytes(): Uint8Array {
    const h = head ?? EMPTY;
    if (bytes.length === 0) return h;
    if (h.length === 0) return bytes;
    const out = new Uint8Array(h.length + bytes.length);
    out.set(h, 0);
    out.set(bytes, h.length);
    return out;
  }

  const sideWord = $derived(side === 'req' ? 'request' : 'response');
</script>

{#snippet clipNote()}
  {#if clipped}
    <div class="clip" data-testid={`${testidPrefix}-clipped`}>
      <span>Showing the first {humanBytes(RENDER_LIMIT)} of {humanBytes(bytes.length)}.</span>
      <Button size="sm" variant="ghost" data-testid={`${testidPrefix}-show-all`} onclick={() => (showAll = true)}
        >Show all</Button
      >
    </div>
  {/if}
{/snippet}

{#snippet utf8Note()}
  <p class="note" data-testid={`${ids.pretty}-utf8`}>
    Body contains invalid UTF-8; shown with replacement characters. Raw shows the bytes exactly.
  </p>
{/snippet}

<div class="viewer" bind:this={root} tabindex="-1" data-keyscope="viewer" data-testid={`${testidPrefix}-viewer`} data-side={side}>
  <div class="bar">
    {@render title?.()}
    <Tabs {tabs} active={tab} {label} onselect={(id) => (chosen = id as ViewerTab)} />
    <span class="spacer"></span>
    <div class="tools" role="toolbar" aria-label={`${label} options`}>
      <IconButton
        size="sm"
        icon="wrap"
        label="Wrap long lines"
        active={wrap}
        aria-pressed={wrap}
        disabled={!textual}
        data-testid={`${testidPrefix}-wrap`}
        onclick={() => (wrap = !wrap)}
      />
      <IconButton
        size="sm"
        icon="pilcrow"
        label="Show CR and LF"
        active={showControls}
        aria-pressed={showControls}
        disabled={!textual}
        data-testid={`${testidPrefix}-controls`}
        onclick={() => (showControls = !showControls)}
      />
      <IconButton
        size="sm"
        icon="search"
        label="Find in message"
        kbd={findHint()}
        active={findOpen}
        aria-pressed={findOpen}
        disabled={!textual}
        data-testid={`${testidPrefix}-find`}
        onclick={() => (findOpen ? closeFind() : openFind())}
      />
    </div>
    {@render actions?.()}
  </div>


  {@render above?.()}

  <!-- The find bar sits right above the text it searches. -->
  {#if findOpen && textual}
    <FindBar
      bind:this={findBar}
      bind:query={findQuery}
      bind:caseSensitive
      count={total}
      capped={search.capped}
      {current}
      onnext={() => (current = nextMatch(total, current))}
      onprev={() => (current = prevMatch(total, current))}
      onclose={closeFind}
      testid={`${testidPrefix}-findbar`}
    />
  {/if}

  <div
    class="panel"
    role="tabpanel"
    aria-label={tab}
    data-testid={ids.panel ? `${ids.panel}-panel-${tab}` : undefined}
  >
    {#if tab === 'pretty'}
      {#if queryRows.length > 0}
        <KvTable rows={queryRows} noun="parameters" label="Query parameters" testid={`${ids.pretty}-query`} />
      {/if}
      {#if body === null}
        {@render bodyStatus?.()}
      {:else if detection.tooLarge}
        <p class="note" data-testid={`${ids.pretty}-note`}>
          No pretty view: the body is {humanBytes(bytes.length)}, over the {humanBytes(PRETTY_LIMIT)} limit. Raw shows it as recorded.
        </p>
      {:else if detection.kind === 'json'}
        {#if prettyText === null}
          <p class="note" data-testid={`${ids.pretty}-note`}>
            The {sideWord} says JSON but the body does not parse. Raw shows it as recorded.
          </p>
        {:else}
          {#if lossy}{@render utf8Note()}{/if}
          {#key prettyText}
            <PrettyJson
              text={prettyText}
              {wrap}
              {showControls}
              matches={matchesFor('pretty')}
              testid={`${ids.pretty}-json`}
            />
          {/key}
        {/if}
      {:else if detection.kind === 'form'}
        {#if lossy}{@render utf8Note()}{/if}
        <KvTable
          rows={formRows}
          total={formTotal}
          limit={KV_LIMIT}
          raw={formRaw}
          label="Form fields"
          testid={`${ids.pretty}-form`}
        />
      {:else if queryRows.length === 0}
        <p class="note" data-testid={`${ids.pretty}-note`}>
          No pretty view: this {sideWord} has no JSON or form body{side === 'req' ? ' and no query string' : ''}.
        </p>
      {/if}
    {:else if tab === 'raw'}
      {#if head}
        <div class="head" data-testid={ids.head} data-exact="true">
          <ByteEditor bytes={head} readonly={true} language="http" wrap={false} {showControls} matches={matchesFor('head')} />
        </div>
      {:else}
        {@render headFallback?.()}
      {/if}
      {#if body === null}
        {@render bodyStatus?.()}
      {:else}
        {#key `${shown.length}-${body.length}`}
          <div data-testid={ids.body}>
            <ByteEditor
              bytes={shown}
              readonly={true}
              diagnostic={bodyDiagnostic}
              language={bodyLanguage}
              {wrap}
              {showControls}
              matches={matchesFor('body')}
            />
          </div>
        {/key}
        {@render clipNote()}
      {/if}
    {:else if tab === 'headers'}
      {#if head}
        <HeadersTable bytes={head} testid={ids.headers} />
      {:else}
        {@render headersFallback?.()}
      {/if}
    {:else}
      {#key `${bytes.length}-${head?.length ?? -1}`}
        <HexWindow bytes={hexBytes()} testid={ids.hex} />
      {/key}
      {#if head === null}
        <p class="note">Body only: this head was not recorded byte-exact.</p>
      {/if}
    {/if}
  </div>
</div>

<style>
  .viewer {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
    min-width: 0;
    min-height: 0;
    flex: 1 1 auto;
  }
  .viewer:focus {
    outline: none;
  }
  /* One row at any pane width: the tab strip gives up width first (it
     scrolls), the toggles and host actions never wrap under it. */
  .bar {
    display: flex;
    align-items: center;
    flex-wrap: nowrap;
    gap: var(--s-2);
    min-width: 0;
  }
  .bar > :global(*) {
    flex: none;
  }
  .bar > :global(.tabs) {
    flex: 0 1 auto;
    min-width: 0;
  }
  .bar > .spacer {
    flex: 1 1 0;
    min-width: 0;
  }
  .spacer {
    flex: 1 1 auto;
  }
  /* The view toggles sit together as one quiet cluster, set off from the
     host's actions by a hairline. */
  .tools {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    padding-right: var(--s-2);
    border-right: 1px solid var(--border);
  }
  .tools:last-child {
    padding-right: 0;
    border-right: none;
  }
  .panel {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
    min-width: 0;
  }
  .head :global(.cm-host) {
    min-height: 0;
  }
  .note {
    font-size: var(--fs-sm);
    color: var(--fg-faint);
    margin: 0;
  }
  .clip {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    font-size: var(--fs-sm);
    color: var(--warn);
  }
</style>
