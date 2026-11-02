# crosswire-api

The REST and WebSocket API for the Crosswire sidecar backend. It is the
security boundary of the sidecar: it issues bootstrap sessions, mints and
checks scoped API tokens, isolates cookies by port and `Origin`, and
authenticates the WebSocket.

This crate builds the [`axum`] router over an [`AppState`]; the server that
binds the loopback listener and delivers the bootstrap token is
`crosswire serve` in `crosswire-server`. How to call the API is in
[docs/api.md](https://github.com/kalash33/crosswire/blob/main/docs/api.md) and the credential design is in
[ADR-0004](https://github.com/kalash33/crosswire/blob/main/docs/adr/0004-api-credentials.md).

## Entry points

- `AppState` — everything the handlers share (store, repeater, intercept,
  scope, CA, sessions, tokens).
- `router(state) -> axum::Router` — the full API router.
- `bootstrap_token(&state) -> String` — mint the single-use bootstrap token
  the server delivers out of band (never on argv).
- `ApiConfig` — the listener address and the credential lifetimes (idle
  timeout, bootstrap TTL), injectable so tests can shorten them.

## Key types

| type | what it is |
| --- | --- |
| `AppState`, `ApiConfig` | shared handler state; the listener address, credential lifetimes, and the `proxy_addr` and `project` (`ProjectInfo`) that `/health` reports |
| `router`, `router_with_fallback` | the API router, with or without a fallback service (the server mounts the embedded UI there) |
| `Scope`, `Scopes` | the `read` / `send` / `admin` capability a route requires and a credential grants |
| `SessionStore`, `BootstrapStore`, `token_hash` | session cookies, the single-use bootstrap token, and the SHA-256 token lookup |
| `PreviewKey`, `mint`, `verify`, `PreviewTokenError` | the per-run HMAC capability tokens for the sandboxed preview origin |
| `ApiError`, `ApiResult` | the one JSON error shape, `{"error":{"code","message"}}` |
| `document` | the OpenAPI document (`docs/api/openapi.json` is its snapshot) |
| `HealthResponse` | `status`, `schema_version`, `format_version`, plus `proxy_addr` (when a proxy runs) and `project` (name and path; **omitted for a non-loopback peer**, because `/health` needs no credential) |
| `SendBody`, `SendOutcomeDto`, `ChangeDto` | the repeater request, its outcome (`exchange_id`, `scope_state`, `changes`, `skipped`, `cert_override`, `ttfb_ms`, `total_ms`, `redirects`) and one recorded edit (`op`, `reason`, `start`, `end`) |
| `RulesDto`, `HeldDto`, `InterceptStatus`, `ForwardBody` | intercept rules, a held request, the status document and the edit-and-forward body |

## Security model (spec §11.2)

- The only unauthenticated routes are `GET /api/v1/ca.der`,
  `GET /api/v1/ca.pem`, `POST /api/v1/session`, `GET /api/v1/openapi.json`
  and `GET /api/v1/health`.
- Every other route needs the session cookie `cw_sid_<port>` (`HttpOnly`,
  `SameSite=Strict`, `Path=/api`) or an `Authorization: Bearer` API token.
- A cookie-authenticated request (including the WebSocket upgrade) must carry
  `Sec-Fetch-Site: same-origin` or an `Origin` exactly equal to the API
  origin, else `403`.
- The API is served only for the `Host`s `localhost[:port]` and
  `127.0.0.1[:port]`; anything else is `421`. No `Access-Control-*` header is
  ever emitted.
- API tokens are `read`, `send` or `admin` scoped (`admin ⊇ send ⊇ read`),
  hashed server-side (SHA-256 of 32 random bytes), shown once, and revocable.
  Bootstrap tokens and session ids are compared in constant time; token
  secrets never enter a plaintext comparison (they are looked up by hash).
- Authentication failures are rate-limited to 10/min per peer and audited.
- `GET /preview/{id}?token=...` is the one non-`/api/v1` route. It serves a
  captured body only on the `127.0.0.1` or `[::1]` origin (any other `Host`,
  `localhost` included, is a bare `404`), authenticated by a per-exchange HMAC
  token bound to the body bytes (60 s lifetime, minted by
  `GET /api/v1/exchanges/{id}/preview-token`), under a deny-all CSP with no
  `Set-Cookie` or `Access-Control-*` header.
- `GET /api/v1/health` reports `project` (name and absolute path) only to a
  loopback peer. `proxy_addr` goes to every peer.
- Every response carries `Crosswire-Schema-Version`, including the gate's own
  refusals.

## Middleware order

Host check (`421`) → route allowlist → credential (cookie with the
same-origin/`Origin` check, or a bearer token looked up by hash) → scope check
(`403 insufficient_scope`) → idle-expiry refresh.

## OpenAPI

The document is generated from the annotated routes via [`utoipa`]. The
committed snapshot lives at `docs/api/openapi.json`; the
`openapi_snapshot_is_current` test fails on drift. Regenerate with:

```sh
UPDATE_OPENAPI=1 cargo test -p crosswire-api --test openapi
```

[`axum`]: https://docs.rs/axum
[`utoipa`]: https://docs.rs/utoipa
[`AppState`]: crate::AppState
