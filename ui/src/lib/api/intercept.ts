// Typed wrappers over the intercept routes (crates/crosswire-api/src/routes/intercept.rs).
// openapi.json leaves these untyped, so this module owns their TS types.
// Every string here (host/method/path, rule strings) is attacker-influenced:
// render as text only (§11.1).
import { apiGet, apiGetBytes, apiSend } from './client';
import { ApiError } from './errors';

export interface HeldDto {
  id: string;
  host: string;
  method: string;
  path: string;
  size: number;
  /** false for an over-limit body: the server has no buffered body to edit. */
  editable: boolean;
}

export interface RulesDto {
  skip_extensions: string[];
  skip_out_of_scope: boolean;
  overflow: 'forward-newest' | 'forward-oldest';
  on_timeout: 'forward' | 'respond-504';
}

export interface InterceptStatus {
  enabled: boolean;
  rules: RulesDto;
  held: HeldDto[];
}

/** Standard base64. An edited `body` REQUIRES an edited `head` (server 422). */
export interface ForwardBody {
  head?: string;
  body?: string;
}

const BASE = '/api/v1/intercept';
const held = (id: string) => `${BASE}/held/${encodeURIComponent(id)}`;

export function getStatus(): Promise<InterceptStatus> {
  return apiGet<InterceptStatus>(BASE);
}
export function putEnabled(enabled: boolean): Promise<void> {
  return apiSend<void>('PUT', `${BASE}/enabled`, { enabled });
}
export function putRules(rules: RulesDto): Promise<void> {
  return apiSend<void>('PUT', `${BASE}/rules`, rules);
}
/** `body` undefined forwards the ORIGINAL bytes. */
export function postForward(id: string, body?: ForwardBody): Promise<void> {
  return apiSend<void>('POST', `${held(id)}/forward`, body);
}
export function postDrop(id: string): Promise<void> {
  return apiSend<void>('POST', `${held(id)}/drop`);
}
export function postForwardAll(): Promise<void> {
  return apiSend<void>('POST', `${BASE}/forward-all`);
}
export async function getHeadBytes(id: string): Promise<Uint8Array> {
  return (await apiGetBytes(`${held(id)}/head`)).bytes;
}
/** null when the held request has no buffered body (404). */
export async function getBodyBytes(id: string): Promise<Uint8Array | null> {
  try {
    return (await apiGetBytes(`${held(id)}/body`)).bytes;
  } catch (e) {
    if (e instanceof ApiError && e.status === 404) return null;
    throw e;
  }
}
