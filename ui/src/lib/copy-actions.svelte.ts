// The copy actions shared by the History selection toolbar, the row context
// menu and the detail's Copy menu. curl and raw go through the audited server
// routes; the URL is built client-side (copy-url.ts). Every result is written
// with the async Clipboard API and never inserted into the DOM; the status
// line and toast show only fixed wording.

import { apiGet } from './api/client';
import { curl, raw } from './api/copy';
import { bytesToLatin1 } from './editor/encoding';
import { connFacts, urlFor, type UrlSource } from './copy-url';
import { keymap, type Action } from './keymap';
import { toasts } from './ui/toasts.svelte';
import type { MenuItem } from './ui/Menu.svelte';
import type { ExchangeDto } from './stores/history.svelte';

export type CopyKind = 'curl' | 'raw';

let status = $state<string | null>(null);

/** The last copy result, shown by the detail's `copy-status` line. */
export const copyStatus = {
  get text(): string | null {
    return status;
  },
  clear(): void {
    status = null;
  },
};

function done(text: string, ok: boolean): void {
  status = text;
  toasts.push({ kind: ok ? 'success' : 'error', text });
}

function failText(e: unknown): string {
  return `Copy failed: ${e instanceof Error ? e.message : 'clipboard unavailable'}`;
}

/** Copy curl or the raw request of exchange `id`. */
export async function copyExchange(id: string, kind: CopyKind, redacted: boolean): Promise<void> {
  try {
    const text =
      kind === 'curl' ? await curl(id, { redacted }) : bytesToLatin1(await raw(id, { redacted }));
    await navigator.clipboard.writeText(text);
    done(`Copied ${kind}${redacted ? ' (redacted)' : ''}.`, true);
  } catch (e) {
    done(failText(e), false);
  }
}

/**
 * Copy the exchange's URL. The row has no TLS flag, so the scheme comes from
 * the full exchange's `request.connection`; if that read fails the URL is
 * built from the row alone (scheme inferred from port 443).
 */
export async function copyUrl(id: string, row: UrlSource): Promise<void> {
  let src: UrlSource = row;
  try {
    const ex = await apiGet<ExchangeDto>(`/api/v1/exchanges/${encodeURIComponent(id)}`);
    src = { ...row, ...connFacts(ex.request?.connection) };
  } catch {
    // Fall back to the row's own host/port.
  }
  try {
    await navigator.clipboard.writeText(urlFor(src));
    done('Copied URL.', true);
  } catch (e) {
    done(failText(e), false);
  }
}

/** Display form of an action's chord, e.g. "Ctrl+Shift+C". */
export function chordLabel(action: Action): string | undefined {
  const c = keymap.chordFor(action);
  if (!c) return undefined;
  return c
    .split('+')
    .map((p) => (p.length === 1 ? p.toUpperCase() : p[0].toUpperCase() + p.slice(1)))
    .join('+');
}

/** The five copy items, with the testids the e2e specs use. */
export function copyItems(id: string, row: UrlSource | null): MenuItem[] {
  return [
    {
      label: 'Copy as curl',
      icon: 'copy',
      kbd: chordLabel('copy-as-curl'),
      testid: 'copy-curl',
      onselect: () => void copyExchange(id, 'curl', false),
    },
    {
      label: 'Copy as curl (redacted)',
      testid: 'copy-curl-redacted',
      onselect: () => void copyExchange(id, 'curl', true),
    },
    {
      label: 'Copy raw request',
      kbd: chordLabel('copy-raw'),
      testid: 'copy-request-raw',
      onselect: () => void copyExchange(id, 'raw', false),
    },
    {
      label: 'Copy raw request (redacted)',
      testid: 'copy-request-raw-redacted',
      onselect: () => void copyExchange(id, 'raw', true),
    },
    {
      label: 'Copy URL',
      testid: 'copy-url',
      disabled: row === null,
      onselect: () => {
        if (row) void copyUrl(id, row);
      },
    },
  ];
}
