// Hand-written mirror of the 11 named component schemas in
// docs/api/openapi.json (the only response/request shapes the document types).
// Exchange rows, scope, and WebSocket events are NOT typed there; their
// consuming modules define them from the server DTOs / spec.
// client.test.ts loads the real document and checks MIRRORED_SCHEMAS against it.

export const MIRRORED_SCHEMAS = [
  'CountsDto',
  'CreateBody',
  'EnabledBody',
  'ForwardBody',
  'HealthResponse',
  'ImportBody',
  'PatchBody',
  'RulesDto',
  'SendBody',
  'SessionRequest',
  'SessionResponse',
] as const;

/** Project totals. */
export interface CountsDto {
  bytes_inline: number;
  bytes_packs: number;
  committed: number;
  error: number;
  exchanges: number;
  history: number;
  intruder: number;
  markers: number;
  repeater: number;
}

/** The `POST /tokens` body. */
export interface CreateBody {
  name: string;
  /** `read`, `send`, `admin`. */
  scopes: string[];
}

/** The `PUT /intercept/enabled` body. */
export interface EnabledBody {
  enabled: boolean;
}

/** The `POST /intercept/held/{id}/forward` body. Base64 (standard). */
export interface ForwardBody {
  body?: string | null;
  head?: string | null;
}

/** The health response body. */
export interface HealthResponse {
  /** Always `"ok"` while the API is serving. */
  status: string;
  /** The wire-format version of recorded Messages and Exchanges. */
  schema_version: number;
  /** The project database format (`user_version`) this build reads and writes. */
  format_version: number;
  /** The proxy's bound address as `host:port` (`[v6]:port` for IPv6), when known. */
  proxy_addr?: string;
}

/** The `POST /scope/import` body. */
export interface ImportBody {
  text: string;
}

/** The `PATCH /exchanges/{id}` body. */
export interface PatchBody {
  notes?: string | null;
  tags?: string[] | null;
}

/** The intercept rules, over the wire. */
export interface RulesDto {
  /** `forward` or `respond-504`. */
  on_timeout: string;
  /** `forward-newest` or `forward-oldest`. */
  overflow: string;
  skip_extensions: string[];
  skip_out_of_scope: boolean;
}

/** The `POST /repeater/send` body. */
export interface SendBody {
  allow_out_of_scope?: boolean;
  /** `off` (default), `in-scope` or `always`. */
  follow?: string | null;
  host: string;
  /** `assisted` or `raw` (default `raw`). */
  mode?: string | null;
  port: number;
  /** Raw request bytes, base64 (standard). */
  raw: string;
  reuse?: boolean;
  sni?: string | null;
  source?: string | null;
  tls?: boolean;
  /** `web-pki` (default) or `accept-invalid`. */
  verify?: string | null;
}

/** The bootstrap request body. */
export interface SessionRequest {
  bootstrap: string;
}

/** The bootstrap response body. */
export interface SessionResponse {
  /** Seconds until the session expires if idle. */
  expires_in: number;
}

/**
 * One recorded edit from a repeater send (`SendOutcomeDto.changes`), from
 * `routes/repeater.rs`. Not in openapi.json (responses are untyped there).
 */
export interface ChangeDto {
  /** What was changed, e.g. "content-length" / "line-endings". */
  op: string;
  /** Why, e.g. "recomputed" / "normalized to CRLF". */
  reason: string;
  /** Byte offset where the change starts (in the sent bytes). */
  start: number;
  /** Byte offset where the change ends. */
  end: number;
}
