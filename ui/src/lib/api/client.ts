import { ApiError, errorFromResponse } from './errors';

export type QueryValue = string | number | boolean | null | undefined;
export type QueryParams = Record<string, QueryValue | readonly QueryValue[]>;

/** Serialize params: arrays repeat the key; null/undefined are skipped. */
function toQuery(params?: QueryParams): string {
  if (!params) return '';
  const qs = new URLSearchParams();
  for (const [key, value] of Object.entries(params)) {
    const items = Array.isArray(value) ? (value as readonly QueryValue[]) : [value as QueryValue];
    for (const item of items) {
      if (item !== undefined && item !== null) qs.append(key, String(item));
    }
  }
  const s = qs.toString();
  return s ? `?${s}` : '';
}

async function request<T>(url: string, init: RequestInit): Promise<T> {
  // Same-origin only: relative URLs, cookie auth, and the Fetch-Metadata
  // header the server's cookie-isolation gate expects.
  const headers = new Headers(init.headers);
  headers.set('Sec-Fetch-Site', 'same-origin');
  const res = await fetch(url, { ...init, headers, credentials: 'same-origin' });
  const text = await res.text();
  if (!res.ok) throw errorFromResponse(res.status, res.statusText, text);
  if (text === '') return undefined as T;
  try {
    return JSON.parse(text) as T;
  } catch {
    throw new ApiError('invalid_response', 'the server returned a non-JSON success body', res.status);
  }
}

export function apiGet<T>(path: string, params?: QueryParams): Promise<T> {
  return request<T>(path + toQuery(params), { method: 'GET' });
}

/**
 * Raw-bytes GET for body endpoints (`GET /exchanges/{id}/body/{side}`), which
 * return `application/octet-stream`, not JSON. Reads the payload as an exact
 * `Uint8Array` and the `x-crosswire-truncated-at` header (null when absent).
 * Reuses the JSON client's error handling for a non-2xx response.
 */
export async function apiGetBytes(
  path: string,
): Promise<{ bytes: Uint8Array; truncatedAt: number | null }> {
  const res = await fetch(path, {
    method: 'GET',
    credentials: 'same-origin',
    headers: { 'Sec-Fetch-Site': 'same-origin' },
  });
  if (!res.ok) {
    const text = await res.text();
    throw errorFromResponse(res.status, res.statusText, text);
  }
  const bytes = new Uint8Array(await res.arrayBuffer());
  const raw = res.headers.get('x-crosswire-truncated-at');
  const truncatedAt = raw === null || raw === '' ? null : Number(raw);
  return { bytes, truncatedAt };
}

export function apiSend<T>(method: string, path: string, body?: unknown): Promise<T> {
  const headers = new Headers();
  const init: RequestInit = { method, headers };
  if (body !== undefined) {
    headers.set('Content-Type', 'application/json');
    init.body = JSON.stringify(body);
  }
  return request<T>(path, init);
}
