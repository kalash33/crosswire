// Typed wrappers over the repeater routes (crates/crosswire-api/src/routes/repeater.rs).
// openapi.json leaves these responses untyped, so this module owns their TS types.
import { apiGet, apiSend } from './client';
import type { ChangeDto, SendBody } from './types';

// `SendBody` is mirrored from openapi.json in types.ts (a test pins it to the
// document). NOTE: on the wire `raw` is standard base64 of the request bytes
// (the handler base64-decodes it), not a Latin-1 string.
export type { ChangeDto, SendBody };

/** The `POST /repeater/send` response. */
export interface SendOutcomeDto {
  exchange_id: string;
  scope_state: string;
  changes: ChangeDto[];
  skipped: string[];
  cert_override: boolean;
  ttfb_ms: number;
  total_ms: number;
  redirects: string[];
}

/** One saved tab draft. `json` is opaque to the server (max 1 MiB). */
export interface DraftDto {
  tab_id: string;
  json: unknown;
  updated_at: number;
}

const TABS = '/api/v1/repeater/tabs';

export function sendRequest(body: SendBody): Promise<SendOutcomeDto> {
  return apiSend<SendOutcomeDto>('POST', '/api/v1/repeater/send', body);
}

export function listTabs(): Promise<DraftDto[]> {
  return apiGet<DraftDto[]>(TABS);
}

export function getTab(id: string): Promise<unknown> {
  return apiGet<unknown>(`${TABS}/${encodeURIComponent(id)}`);
}

export function putTab(id: string, json: unknown): Promise<void> {
  return apiSend<void>('PUT', `${TABS}/${encodeURIComponent(id)}`, json);
}

export function deleteTab(id: string): Promise<void> {
  return apiSend<void>('DELETE', `${TABS}/${encodeURIComponent(id)}`);
}
