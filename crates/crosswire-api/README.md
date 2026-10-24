# crosswire-api

The REST and WebSocket API for the Crosswire sidecar backend. It is the
security boundary of the sidecar: it issues bootstrap sessions, mints and
checks scoped API tokens, isolates cookies by port and `Origin`, and
authenticates the WebSocket.

This crate builds the [`axum`] router over an [`AppState`]; the server that
binds the loopback listener and delivers the bootstrap token lives in a
separate task (`crosswire serve`).

## Entry points

- `AppState` — everything the handlers share (store, repeater, intercept,
  scope, CA, sessions, tokens).
- `router(state) -> axum::Router` — the full API router.
- `bootstrap_token(&state) -> String` — mint the single-use bootstrap token
  the server delivers out of band (never on argv).
- `ApiConfig` — the listener address and the credential lifetimes (idle
  timeout, bootstrap TTL), injectable so tests can shorten them.

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
