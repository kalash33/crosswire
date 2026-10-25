import { errorFromResponse } from './errors';

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
  return JSON.parse(text) as T;
}

export function apiGet<T>(path: string, params?: QueryParams): Promise<T> {
  return request<T>(path + toQuery(params), { method: 'GET' });
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
