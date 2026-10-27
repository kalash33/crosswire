import { apiGetBytes } from './client';
import { ApiError, errorFromResponse } from './errors';

export type CopyShell = 'sh' | 'pwsh';

/** The OS picks the shell dialect: PowerShell on Windows, POSIX sh elsewhere. */
export function defaultShell(): CopyShell {
  const nav = typeof navigator === 'undefined' ? null : navigator;
  const plat = `${nav?.platform ?? ''} ${nav?.userAgent ?? ''}`;
  return /win/i.test(nav?.platform ?? '') || /windows/i.test(plat) ? 'pwsh' : 'sh';
}

/** `GET /exchanges/{id}/curl` (text/plain). Audited server-side. */
export async function curl(
  id: string,
  opts: { shell?: CopyShell; redacted?: boolean } = {},
): Promise<string> {
  const qs = new URLSearchParams({
    shell: opts.shell ?? defaultShell(),
    redacted: String(opts.redacted ?? false),
  });
  const res = await fetch(`/api/v1/exchanges/${encodeURIComponent(id)}/curl?${qs}`, {
    method: 'GET',
    credentials: 'same-origin',
    headers: { 'Sec-Fetch-Site': 'same-origin' },
  });
  const text = await res.text();
  if (!res.ok) throw errorFromResponse(res.status, res.statusText, text);
  return text;
}

/** `GET /exchanges/{id}/raw` (octet-stream): the exact request bytes. */
export async function raw(
  id: string,
  opts: { redacted?: boolean } = {},
): Promise<Uint8Array> {
  const qs = new URLSearchParams({ redacted: String(opts.redacted ?? false) });
  try {
    const { bytes } = await apiGetBytes(`/api/v1/exchanges/${encodeURIComponent(id)}/raw?${qs}`);
    return bytes;
  } catch (e) {
    throw e instanceof ApiError ? e : new ApiError('copy_failed', 'raw copy failed', 0);
  }
}
