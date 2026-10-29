# Crosswire preview zero-egress layer — design spec

> **Status:** the last unbuilt §13 security gate (#4, preview zero-egress). Builds the
> §11.1 untrusted-HTML/image preview isolation. Authority: the product spec
> `docs/superpowers/specs/2026-10-22-crosswire-design.md` (§11.1, §11.2, §13). Where
> this document and the product spec disagree, the product spec wins.

## 1. Goal

Let the operator view a captured response body as rendered HTML or an image **without
any risk of the untrusted content exfiltrating data, stealing the session, or reaching
the network**. Previews are served from a different browser origin than the UI/API (so
the session cookie never attaches), in a sandboxed iframe with a deny-all CSP, gated by
a short-lived per-exchange capability token, and proven inert by a zero-egress corpus.

## 2. The isolation model (§11.1 / §11.2)

- The server binds `127.0.0.1` (already does for api + proxy). The browser treats
  `http://localhost:<port>` and `http://127.0.0.1:<port>` as **different origins**, so a
  cookie set for the `localhost` UI is **never** sent to a `127.0.0.1` preview.
- The Host check becomes **route-dependent**: the existing UI/API routes keep requiring
  `Host: localhost` (the host-421 gate, `crosswire-api` auth); the **preview route is
  served only when `Host` is `127.0.0.1` or `[::1]`** and is refused (404/421) on
  `localhost`. This keeps the two origins disjoint server-side even though they share a
  port.
- The preview route is **unauthenticated by cookie**; access is a per-exchange HMAC
  capability token instead. It is the **fourth unauthenticated route** in the route-auth
  test (after static assets, session bootstrap, and the CA public cert).

## 3. The capability token (§11.1)

- `token = base64url( expiry_be_u64 || HMAC_SHA256(key, exchange_ulid || body_hash || expiry) )`,
  where `key` is a per-`serve` random secret (≥256 bits, never persisted, never in any
  API response), `body_hash` is the stored body's content hash, and `expiry` is
  `now + 60 s`.
- Verification (constant-time HMAC compare): the token is valid only for GET, only for
  that exchange ULID + current body hash, and only before `expiry`. An expired, tampered,
  or wrong-exchange token is 403 (stable error envelope on the API path; a minimal
  non-leaking 403 on the preview origin). GET only — any other method is 405.
- An **authenticated** mint route issues the token to the UI: `GET /api/v1/exchanges/{id}/preview-token`
  (Read scope, cookie-auth on the `localhost` origin), returning `{ token, path, expires_in }`
  where `path` is the preview URL path. Minting is audited (§9.4) as `preview.mint`.

## 4. The preview route (§11.1)

- `GET /preview/{id}?token=...` on the preview (`127.0.0.1`/`[::1]`) origin only.
- On a valid token it returns the exchange's **response body** bytes with the body's own
  content-type (so HTML renders, an image shows), and these headers:
  - `Content-Security-Policy: sandbox; default-src 'none'; img-src data: blob:`
  - `Referrer-Policy: no-referrer`
  - `X-Content-Type-Options: nosniff`
  - no `Set-Cookie`, no `Access-Control-*`.
- Blob/pack bodies are read through the existing body-read path; a body over a preview
  size cap (reuse `capture_cap` / a preview cap) is truncated with a banner, matching the
  existing body-serving limits. A missing/!ULID id or bad token is a minimal 403/404 that
  leaks no project data.
- The route does **no** cookie/session handling and never emits CORS headers.

## 5. The UI (§11.1)

- A "Preview" action on a response body: the UI (on `localhost`) calls the mint route,
  gets `{ token, path }`, and renders a **sandboxed iframe** —
  `sandbox` **without** `allow-same-origin` (and without `allow-scripts` unless HTML
  interactivity is explicitly wanted; default: no scripts) — with
  `src = http://127.0.0.1:<port>/preview/{id}?token=...`.
- The UI CSP (`ui.rs`) gains `frame-src http://127.0.0.1:<port>` so the iframe is allowed;
  everything else in the UI CSP stays. The UI never inlines the captured bytes; it only
  points the iframe at the preview origin.
- The preview opens only in the user's default browser (the UI already does), never the
  proxied profile.

## 6. Zero-egress corpus (§11.1 / §13)

A Playwright test (its own spec, real serve or vite-preview with the preview route
mocked to serve corpus bytes) loads, into the sandboxed preview iframe, HTML bodies that
each attempt egress via **every** vector the spec lists: `<img src>`, `<meta http-equiv=refresh>`,
an anchor auto-click, a form auto-submit, CSS `background:url()` and `@import`,
`<link rel=stylesheet/prefetch>`, `<iframe src>`, `<object data>`, `<video poster>`, and
SVG `<use href>`. The test routes ALL network and asserts **zero** requests reach any
origin other than the preview's own `GET /preview/...` (i.e. no external host, no
`localhost` API/cookie origin). This is the §13 **preview zero-egress** gate.

## 7. Scope

**In scope:** the HMAC token (mint + verify), the preview route + Host-origin enforcement
+ headers, the UI mint/iframe wiring + UI-CSP frame-src, the zero-egress corpus, and the
§13 gate wiring (`docs/security-gates.md #4` → proven; the preview route added to the
route-auth test's public allowlist as the fourth route).

**Out of scope:** a non-HTML "pretty" renderer; preview of request bodies (response only
for v0.1 unless trivial); image transcoding; a preview for streamed/blob-only bodies
beyond the existing read path; the decode-view (v0.2a).

## 8. Global constraints (verbatim)

- `unsafe_code = "forbid"`; MSRV 1.88; rustls ring-only; `cargo build` Node-free.
- Conventional Commits, ≤72-char subjects, no attribution trailers.
- `cargo deny` clean; no new external dependency unless unavoidable (HMAC-SHA256: prefer
  an existing dep — `ring`/`hmac`/`sha2` may already be in the graph; justify + lockfile +
  deny-clean if a new one is needed).
- Additive only: no wire/schema change to the stored Message; the mint route + preview
  route are new; the Host check becomes route-dependent (additive to the existing
  localhost enforcement).
- FULL GATE before a task is done: `just lint`, `just doc`, `cargo deny check`,
  `cargo test`, and the UI e2e where the UI changes.
- Reuse the existing route-auth test (`crosswire-api`/`crosswire-server` gates), the
  body-read path, the audit log, and the UI CSP in `ui.rs`.

## 9. Review focus (inputs the tests must cover)

- A token for exchange A must NOT preview exchange B (binding to the ULID).
- A token after the body changed (hash mismatch) is rejected (binding to body hash).
- An expired token (past 60 s) is 403; a tampered token is 403; a non-GET is 405.
- The preview route on `Host: localhost` is refused (origin isolation) — so a page on the
  UI origin cannot load the preview as same-origin.
- The preview response carries the deny-all CSP + no `Set-Cookie` + no CORS, even for an
  HTML body full of egress vectors (the corpus).
- The mint route requires Read auth and is audited; the preview route is the only new
  unauthenticated route (route-auth test still passes with exactly the allowed set).

## 10. Acceptance

- The preview zero-egress gate (corpus → no external request) is green in CI.
- The token binding (exchange + body hash + 60 s TTL + GET-only), the Host-origin
  refusal on localhost, and the deny-all CSP headers are each asserted.
- The route-auth test lists the preview route as the fourth public route and still
  rejects everything else; the mint route requires Read + is audited.
- `docs/security-gates.md #4` → proven.
- `unsafe_code = "forbid"` held; `cargo deny` clean; the CA private key / session cookie
  never reachable from the preview origin (a page on `127.0.0.1` gets no cookie).
