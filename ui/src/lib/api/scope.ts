// Typed wrappers over the scope routes (crates/crosswire-api/src/routes/scope.rs).
// openapi.json leaves these untyped, so this module owns their TS types,
// mirrored from crates/crosswire-scope/src/{rule.rs,input.rs,scope.rs}.
// Hosts, paths and regexes are attacker-influenced: render as text only (§11.1).
import { apiGet, apiSend } from './client';

/** serde `tag="type", content="value", rename_all="kebab-case"`. */
export type HostPattern =
  | { type: 'exact'; value: string }
  | { type: 'suffix'; value: { suffix: string; also_apex: boolean } }
  | { type: 'regex'; value: string }
  | { type: 'ip'; value: string }
  | { type: 'cidr'; value: string };

export type PathMatch = { type: 'prefix'; value: string } | { type: 'regex'; value: string };

/** kebab-case enum. */
export type Scheme = 'http' | 'https';

export interface Rule {
  host: HostPattern;
  /** BTreeSet<u16> -> JSON array; omitted = any port. */
  ports?: number[];
  scheme?: Scheme;
  path?: PathMatch;
}

export interface Scope {
  include: Rule[];
  exclude: Rule[];
}

export const HOST_TYPES = ['exact', 'suffix', 'regex', 'ip', 'cidr'] as const;
export const PATH_TYPES = ['prefix', 'regex'] as const;

export function getScope(): Promise<Scope> {
  return apiGet<Scope>('/api/v1/scope');
}
/** Needs `admin`. 422 `scope_invalid` leaves the live scope untouched. */
export function putScope(scope: Scope): Promise<void> {
  return apiSend<void>('PUT', '/api/v1/scope', scope);
}
/** Needs `admin`. Parses only; nothing is saved. */
export function importScope(text: string): Promise<Scope> {
  return apiSend<Scope>('POST', '/api/v1/scope/import', { text });
}
/** Needs `admin`. */
export function setEnforceStrict(enabled: boolean): Promise<void> {
  return apiSend<void>('PUT', '/api/v1/settings/enforce_scope_strictly', { enabled });
}
