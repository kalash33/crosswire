# Preview Zero-Egress Layer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the §11.1 untrusted-HTML/image preview isolation — the last §13 security gate (#4).

**Architecture:** A per-exchange HMAC capability token gates an unauthenticated preview route served only on the `127.0.0.1`/`[::1]` origin (never `localhost`), returning the captured body in a sandboxed iframe with a deny-all CSP; the browser's cookie scoping keeps the session off the preview origin; a zero-egress Playwright corpus proves no untrusted content escapes.

**Tech Stack:** Rust (crosswire-api, crosswire-server), `ring::hmac` (already in the graph), `sha2`, axum, Svelte/TS UI, Playwright.

**Spec:** docs/superpowers/specs/2026-10-29-crosswire-preview-zero-egress-design.md

## Global Constraints

- `unsafe_code = "forbid"`; MSRV 1.88; rustls ring-only; `cargo build` Node-free.
- Conventional Commits, ≤72-char subjects, **no attribution trailers** (verify `git log -1 | grep -c -i claude` == 0 after each commit).
- `cargo deny` clean; **no new external dependency** — HMAC-SHA256 via `ring::hmac` (already present via rustls); body hash via `sha2`/the store's existing hash.
- Additive only: no stored-Message wire/schema change; the mint + preview routes are new; the Host check becomes route-dependent (additive to the existing loopback enforcement).
- FULL GATE before a task is done: `just lint`, `just doc`, `cargo deny check`, `cargo test`, and the UI e2e where the UI changes.
- Reuse the route-auth test + `is_unauthenticated`/`host_is_loopback`/`allowed_origins` (`crosswire-api/src/auth.rs`), the body-read path, the audit log, the UI CSP (`crosswire-server/src/ui.rs`).

## Review Focus

- A token minted for exchange A must NOT preview exchange B, and a token after the body changed (hash mismatch) is rejected — binding (Task 0).
- An expired (>60 s) / tampered token is 403; a non-GET is 405 — token lifecycle (Task 0/1).
- The preview route on `Host: localhost` is REFUSED (so a page on the UI origin can't load it same-origin), and served on `Host: 127.0.0.1`/`[::1]` — origin isolation (Task 1).
- The preview response carries `CSP: sandbox; default-src 'none'; img-src data: blob:` + no `Set-Cookie` + no CORS, even for an HTML body full of egress vectors (Task 1 + Task 3).
- The route-auth test still rejects everything but the now-four public routes; the mint route requires Read + is audited (Task 1).

---

### Task 0: HMAC capability token + mint route

**Files:**
- Create: `crates/crosswire-api/src/preview_token.rs` (mint + verify)
- Modify: `crates/crosswire-api/src/routes/exchanges.rs` (the mint handler), the router, the OpenAPI snapshot, `crates/crosswire-api/src/lib.rs`/state (hold the per-serve preview key)
- Test: `crates/crosswire-api/tests/` (a new `preview_token.rs` unit+route test)

**Interfaces:**
- Produces:
  - `pub struct PreviewKey([u8; 32])` with `PreviewKey::random() -> Self` (per-serve secret; held on the app state; never serialized).
  - `pub fn mint(key: &PreviewKey, ulid: Ulid, body_hash: &[u8; 32], now: SystemTime, ttl: Duration) -> String` — `base64url(expiry_be_u64 || HMAC_SHA256(key, ulid_bytes || body_hash || expiry_be))`.
  - `pub fn verify(key: &PreviewKey, ulid: Ulid, body_hash: &[u8; 32], token: &str, now: SystemTime) -> Result<(), PreviewTokenError>` — constant-time HMAC compare (`ring::hmac::verify`), checks expiry > now.
  - Route `GET /api/v1/exchanges/{id}/preview-token` (Read scope), returns `{ token, path, expires_in }` where `path = "/preview/{id}?token=..."`, audited `preview.mint`.

- [ ] **Step 1: Write the failing tests** (`preview_token.rs`): `mint_then_verify_ok`; `verify_rejects_wrong_exchange` (mint for ulid A, verify for B → err); `verify_rejects_changed_body_hash`; `verify_rejects_expired` (now > expiry → err); `verify_rejects_tampered` (flip a token byte → err). Route tests: `mint_requires_read_scope` (401 without cred), `mint_returns_token_and_path_and_is_audited` (200 with `{token,path,expires_in: 60}`, an audit `preview.mint` row).
- [ ] **Step 2: Run** `cargo test -p crosswire-api` (new tests) → FAIL.
- [ ] **Step 3: Implement** `preview_token.rs` (`ring::hmac` SHA256; base64url via the existing base64 dep), the `PreviewKey` on app state (generated at serve start), and the mint handler. The body hash comes from the exchange's stored body (reuse the body-read/hash path).
- [ ] **Step 4: Run** `cargo test -p crosswire-api` → green (incl. the OpenAPI snapshot regenerated).
- [ ] **Step 5: FULL GATE.**
- [ ] **Step 6: Commit** `feat(api): per-exchange HMAC preview capability token`.

---

### Task 1: preview route + origin enforcement + deny-all CSP

**Files:**
- Create: `crates/crosswire-api/src/routes/preview.rs` (the preview handler) or add to `routes/exchanges.rs`
- Modify: the router + the Host-check path in `crosswire-api/src/auth.rs` (route-dependent), `crosswire-api/tests/` route-auth test (the fourth public route + the host-origin cases)
- Test: `crates/crosswire-api/tests/` (preview route behavior) + the existing route-auth gate

**Interfaces:**
- Consumes: `preview_token::verify` + `PreviewKey` (Task 0); the body-read path; `is_unauthenticated`/`host_is_loopback` (auth.rs).
- Produces: `GET /preview/{id}?token=...` served ONLY when `Host` is `127.0.0.1`/`[::1]` (NOT `localhost`); on a valid token returns the exchange response body with its content-type + `Content-Security-Policy: sandbox; default-src 'none'; img-src data: blob:` + `Referrer-Policy: no-referrer` + `X-Content-Type-Options: nosniff`, no `Set-Cookie`, no CORS. A new `fn host_is_preview_origin(host) -> bool` (`127.0.0.1`/`[::1]`, not localhost).

- [ ] **Step 1: Write the failing tests:** `preview_served_on_127_with_valid_token` (200 + body + the three headers + no set-cookie); `preview_refused_on_localhost_host` (a `Host: localhost` request to `/preview/..` → 404/421, NOT the body — origin isolation); `preview_bad_token_is_403`; `preview_non_get_is_405`; `preview_unknown_id_is_404`. Route-auth gate: `/preview/{id}` is the FOURTH public route (no credential needed) and every OTHER non-public route still 401s; the mint route is NOT public.
- [ ] **Step 2: Run** `cargo test -p crosswire-api` (+ `-p crosswire-server` route-auth gate) → FAIL.
- [ ] **Step 3: Implement** the preview handler + `host_is_preview_origin` + the route-dependent Host enforcement (preview requires the 127.0.0.1 origin; `/api/v1` + static keep requiring localhost). Serve the body via the existing read path; apply a preview size cap (reuse `capture_cap`) with a truncation banner. Update the route-auth test's public set to the four routes.
- [ ] **Step 4: Run** `cargo test -p crosswire-api -p crosswire-server` → green.
- [ ] **Step 5: FULL GATE.**
- [ ] **Step 6: Commit** `feat(api): sandboxed preview route on the 127.0.0.1 origin`.

---

### Task 2: UI preview action + sandboxed iframe

**Files:**
- Modify: the UI API client (a `previewToken(id)` call), a response-body view component (a "Preview" action + the iframe), `crates/crosswire-server/src/ui.rs` (UI CSP `frame-src`)
- Test: `ui/e2e/` (a preview-wiring spec) + a unit test for the client

**Interfaces:**
- Consumes: `GET /api/v1/exchanges/{id}/preview-token` (Task 0) → `{ token, path }`; the preview route (Task 1).
- Produces: a "Preview" control that mints a token and renders an `<iframe sandbox src="http://127.0.0.1:<port>/preview/{id}?token=...">` — `sandbox` WITHOUT `allow-same-origin` and WITHOUT `allow-scripts` (default: inert render). The UI CSP gains `frame-src http://127.0.0.1:<port>`.

- [ ] **Step 1: Write the failing e2e** (`ui/e2e/preview.spec.ts`, API mocked): clicking "Preview" calls the mint route; the rendered iframe has `sandbox` without `allow-same-origin`/`allow-scripts` and its `src` points at `http://127.0.0.1:<port>/preview/<id>?token=<t>`. A unit test that the UI CSP string includes `frame-src http://127.0.0.1`.
- [ ] **Step 2: Run** `npm --prefix ui run test` + the e2e → FAIL.
- [ ] **Step 3: Implement** the client call, the Preview control + iframe, and the `ui.rs` CSP `frame-src`. The captured bytes are never inlined into the UI DOM — only the iframe src points at the preview origin (the §11.1 lint still passes).
- [ ] **Step 4: Run** `npm --prefix ui run lint`, `npm --prefix ui run test`, `npm --prefix ui run e2e` → green; the Rust UI-CSP test passes.
- [ ] **Step 5: FULL GATE** (incl. UI e2e; confirm `ui-dist-check` / rebuild the committed bundle if the repo commits it).
- [ ] **Step 6: Commit** `feat(ui): sandboxed preview iframe on the preview origin`.

---

### Task 3: zero-egress corpus + §13 gate #4

**Files:**
- Create: `ui/e2e/preview-zero-egress.spec.ts`
- Modify: `docs/security-gates.md` (#4 → proven)

**Interfaces:**
- Consumes: the preview route (Task 1) + the UI iframe (Task 2). The corpus bodies are served to the sandboxed iframe (via the real preview route against a seeded exchange, or the preview route mocked to return the corpus HTML with the real deny-all CSP headers).

- [ ] **Step 1: Write the failing e2e** `preview-zero-egress.spec.ts`: for EACH egress vector — `<img src>`, `<meta http-equiv="refresh">`, an auto-clicked anchor, an auto-submitted form, CSS `background:url()` and `@import`, `<link rel="stylesheet"/prefetch>`, `<iframe src>`, `<object data>`, `<video poster>`, SVG `<use href>` — load an HTML body containing it into the sandboxed preview iframe; route ALL network and assert ZERO requests reach any origin other than the preview's own `/preview/...` (no external host, no `localhost` API origin). Assert the preview response's CSP + Referrer-Policy headers are the deny-all set.
- [ ] **Step 2: Run** the spec → FAIL (or demonstrate it would catch a leak: temporarily relax the iframe sandbox / CSP and confirm a vector escapes; restore).
- [ ] **Step 3: Implement** the corpus spec; ensure it runs in the ubuntu UI-e2e CI job.
- [ ] **Step 4: Run** `npm --prefix ui run e2e` (the new spec) → green (zero external requests across all vectors).
- [ ] **Step 5: FULL GATE**, then update `docs/security-gates.md` #4 (preview zero-egress) → proven, citing the corpus spec + the token/origin/CSP tests.
- [ ] **Step 6: Commit** `test(ui): preview zero-egress corpus; prove gate #4`.

---

## Notes for the executor

- Order 0 → 1 → 2 → 3 (token → route → UI → corpus). Task 1 consumes Task 0; Task 2 consumes both; Task 3 consumes 1+2.
- Suggested models: Task 0 (HMAC/token security) and Task 1 (origin enforcement, the fourth public route — security-correctness) → opus; Task 2 (UI) and Task 3 (Playwright corpus) → sonnet.
- The isolation is browser-enforced (cookie scoping to the localhost origin) PLUS server-enforced (preview served only on the 127.0.0.1 origin, HMAC-gated, deny-all CSP). Both must hold; the corpus proves the content is inert even if a token leaks.
- If a corpus vector escapes, that's a real isolation defect — STOP and report (do not relax the CSP/sandbox to pass).
