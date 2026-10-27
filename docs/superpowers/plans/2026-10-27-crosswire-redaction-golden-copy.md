# Redaction Golden + copy-as-curl / copy-raw Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close §13 security gate #14 (redaction golden) and ship the §7.6 copy-as-curl and copy-raw actions.

**Architecture:** Generate curl and raw-request bytes **server-side** in `crosswire-tools` so the documented shell-quoting routine is the routine the golden tests and redaction can't be client-bypassed; extend `RedactionPolicy` to redact inline bodies so a body-borne secret is caught; expose two audited GET routes; wire Burp-shortcut-unredacted + explicit-redacted clipboard actions in the UI; land a golden gate over a real `crosswire serve`.

**Tech Stack:** Rust (crosswire-model, crosswire-tools, crosswire-api, crosswire-server), Svelte/TS UI, axum, rusqlite, Playwright.

**Spec:** docs/superpowers/specs/2026-10-27-crosswire-redaction-golden-copy-design.md

## Global Constraints

- `unsafe_code = "forbid"` workspace-wide; MSRV 1.88; `cargo build` Node-free.
- Conventional Commits, ≤72-char subjects, **no attribution trailers** (verify `git log -1 | grep -c -i claude` == 0 after each commit).
- `cargo deny` clean; any new dep justified and in the lockfile (none expected).
- Additive only: no wire-format, schema, or query-semantics change.
- FULL GATE before a task is done: `just lint`, `just doc`, `cargo deny check`, `cargo test` (+ `--features synthetic` where relevant), and the UI e2e where the UI changes. `just test` passing locally is not enough (env/platform checks live in the other gates).
- Redaction placeholder text and the default policy set are unchanged; this plan only adds a new *consumer* (inline body) and new *callers* (copy routes).

## Review Focus

- A secret spanning a header **and** an inline body — both redacted independently (Task 0 + Task 4).
- A value pattern at the very end of the body buffer, no trailing delimiter (Task 0).
- A request with **no** body — curl has no `--data-binary`; raw is head-only (Task 1).
- A `Body::Blob` exchange — curl/raw omit the body out-of-band, no bytes leak, no panic (Task 1).
- A tunnel/CONNECT exchange — nothing to redact; curl route returns a typed error, not a panic (Task 1 + Task 2).

---

### Task 0: Inline-body redaction in `RedactionPolicy`

**Files:**
- Modify: `crates/crosswire-model/src/redact.rs` (extend `redact_message`; the body-scrub reuses the existing `find_matches` / `replace_ranges` path that `scrub` uses)
- Test: `crates/crosswire-model/tests/redaction.rs`

**Interfaces:**
- Consumes: existing `RedactionPolicy::{redact_message, value_patterns}`, `Body::Inline { .. }` / `Body::Blob { .. }` (message.rs:307).
- Produces: `redact_message(&Message) -> Message` now also returns a message whose `Body::Inline` bytes have had the `value_patterns` applied (header names do **not** apply to a body); `Body::Blob` is returned unchanged.

- [ ] **Step 1: Write the failing tests** in `redaction.rs`:
  - `inline_body_value_patterns_are_redacted`: a message with `Body::Inline` bytes containing `access_token=SENTINEL1` and `api_key=SENTINEL2` → after `default_policy().redact_message`, the inline body contains neither `SENTINEL1` nor `SENTINEL2`.
  - `inline_body_pattern_at_end_of_buffer`: body ends exactly with `api_key=SENTINEL` (no trailing byte) → redacted.
  - `blob_body_is_unchanged`: a `Body::Blob` message is returned with its body reference identical (blob redaction is out of scope).
  - `header_names_do_not_apply_to_body`: a body containing the literal text `Cookie: x` (a header *name*, not a value pattern) is left intact (only value patterns touch bodies).
- [ ] **Step 2: Run** `cargo test -p crosswire-model --test redaction` → the four new tests FAIL.
- [ ] **Step 3: Implement** the body branch in `redact_message` (or a helper it calls): when the message body is `Body::Inline`, replace the inline bytes with the value-pattern scrub of those bytes; leave `Body::Blob` untouched. Update the `redact.rs` type/doc comment that currently says "Inline bodies are **not** redacted yet."
- [ ] **Step 4: Run** `cargo test -p crosswire-model` → all green.
- [ ] **Step 5: FULL GATE** (`just lint`, `just doc`, `cargo deny check`, `cargo test`).
- [ ] **Step 6: Commit** `feat(model): redact value patterns in inline bodies`.

---

### Task 1: curl + raw rendering in `crosswire-tools`

**Files:**
- Create: `crates/crosswire-tools/src/copy.rs` (register `mod copy;` + re-exports in `crates/crosswire-tools/src/lib.rs`)
- Test: `crates/crosswire-tools/tests/copy.rs`

**Interfaces:**
- Consumes: `Message` accessors (`head`, `body`, `target_addr`, headers), `RedactionPolicy::redact_message` (Task 0), the request message of an `Exchange` (`Exchange::request`).
- Produces:
  - `pub enum Shell { Posix, PowerShell }`
  - `pub fn to_curl(req: &Message, shell: Shell) -> String`
  - `pub fn to_raw(req: &Message) -> Vec<u8>` (head bytes + inline body bytes; a `Body::Blob` contributes no bytes)
  - `pub fn quote(arg: &[u8], shell: Shell) -> String` (the documented single-quoting routine; its own public fn so the golden can target it)
  - a boolean the API can read to flag an omitted blob body — expose via `pub fn body_is_blob(req: &Message) -> bool` (keeps the render fns pure).

- [ ] **Step 1: Write the failing tests** in `copy.rs`:
  - `posix_quote_is_total`: `quote(b"a'b$(id)`\nc;d", Posix)` equals `'a'\''b$(id)` + backtick + newline + `c;d'` (single-quote wrap, each `'` → `'\''`); re-splitting the quoted string under POSIX word rules yields exactly the original bytes as one field. (Assert structurally — do NOT execute a shell.)
  - `powershell_quote_doubles_quotes`: embedded `'` becomes `''` inside a single-quoted string.
  - `curl_get_has_no_data_flag`: a GET request → the curl string contains no `--data-binary` and no `-d`.
  - `curl_post_uses_data_binary`: a POST with an inline body → the body appears via `--data-binary`, and every header is a `-H` with a quoted value.
  - `curl_blob_body_is_omitted_not_leaked`: a `Body::Blob` POST → the curl string contains a `# body omitted` comment and none of the (unavailable) body bytes; `body_is_blob` is true.
  - `to_raw_head_plus_inline_body`: raw of a request with an inline body = head bytes followed by the body bytes; a no-body request = head bytes only.
  - `redacted_render_has_no_secret`: `to_curl(&policy.redact_message(req), Posix)` and `to_raw(&policy.redact_message(req))` for a req with `Authorization: Bearer <jwt>` and body `access_token=SENTINEL` contain neither the bearer token nor `SENTINEL`; the unredacted renders contain both.
- [ ] **Step 2: Run** `cargo test -p crosswire-tools --test copy` → FAIL.
- [ ] **Step 3: Implement** `copy.rs`. URL = scheme (`https` when the exchange was recorded over TLS, else `http`) + `target_addr` + request-target; method via `-X` when not GET; headers as `-H 'Name: value'`; inline body as `--data-binary`; blob body → a `# body omitted (N bytes in pack)` comment line. Header order preserves the recorded order. No locale/env dependence. The quoting rule is documented in a doc comment per shell stating why it is total.
- [ ] **Step 4: Run** `cargo test -p crosswire-tools` → green.
- [ ] **Step 5: FULL GATE.**
- [ ] **Step 6: Commit** `feat(tools): render a request as curl and as raw bytes`.

---

### Task 2: audited `/curl` and `/raw` API routes

**Files:**
- Modify: `crates/crosswire-api/src/routes/exchanges.rs` (two handlers), the router registration, and the OpenAPI doc list (an `openapi_snapshot_is_current`-style test guards it — regenerate the snapshot)
- Test: `crates/crosswire-api/tests/` (a new `copy.rs` or extend an existing exchanges test)

**Interfaces:**
- Consumes: `to_curl` / `to_raw` / `Shell` / `body_is_blob` (Task 1), `RedactionPolicy::redact_message` (Task 0), the store's exchange lookup used by the existing `/body/{side}` handler (exchanges.rs:269), `state.store.audit(actor, action, target, result)` (4 string args), the Read-scope auth extractor used by other GET routes.
- Produces:
  - `GET /api/v1/exchanges/{id}/curl?shell=sh|pwsh&redacted=bool` → `text/plain`; default `shell=sh`, `redacted=false`.
  - `GET /api/v1/exchanges/{id}/raw?redacted=bool` → `application/octet-stream`.
  - Each writes an audit row: action `copy.curl` / `copy.raw`, target = exchange ULID, result = the variant string (`redacted` or `unredacted`, suffixed with the shell for curl, e.g. `redacted:pwsh`).

- [ ] **Step 1: Write the failing tests:**
  - `curl_route_redacted_vs_plain`: seed one exchange with a secret header; `redacted=true` body has no secret, `redacted=false` has it; `shell=pwsh` differs from `sh` on quoting; content-type `text/plain`.
  - `raw_route_redacted_vs_plain`: same for `/raw`, octet-stream.
  - `copy_is_audited_with_variant`: after a redacted curl GET and a plain raw GET, `audit.jsonl` has a `copy.curl` row with a `redacted` result and a `copy.raw` row with an `unredacted` result.
  - `curl_bad_shell_is_400` and `curl_unknown_id_is_404`: the stable error envelope, not a panic.
  - `copy_requires_read_scope`: no credential → 401 (the route-auth walk already covers new routes; confirm these two are not accidentally in the public allowlist).
- [ ] **Step 2: Run** the api test → FAIL.
- [ ] **Step 3: Implement** the handlers (mirror `/body/{side}` for lookup and auth), the router lines, and regenerate the OpenAPI snapshot. Add `copy.curl` / `copy.raw` to any audited-action documentation list.
- [ ] **Step 4: Run** `cargo test -p crosswire-api` → green (including the OpenAPI snapshot and the existing `route_auth` / `read_token_cannot_mutate` gates — these are GET read routes, so a read token may call them; confirm they are **not** added to the mutation list).
- [ ] **Step 5: FULL GATE.**
- [ ] **Step 6: Commit** `feat(api): audited curl and raw copy routes`.

---

### Task 3: UI copy actions + keymap

**Files:**
- Modify: `ui/src/lib/keymap.ts` (add `copy-as-curl` and `copy-raw` to the reserved action list and `DEFAULT_BINDINGS`), the request-detail component(s) used by history / repeater / intercept (a shared "copy menu" is preferable to three copies), and the API client
- Test: `ui/e2e/copy.spec.ts` (new) + `ui/src/lib/keymap.test.ts` (binding present)

**Interfaces:**
- Consumes: the `/curl` and `/raw` routes (Task 2); the existing keymap dispatch (`DEFAULT_BINDINGS`, central dispatcher) and API-client pattern.
- Produces: a copy control offering **Copy as curl** (Burp shortcut → unredacted; the OS picks `sh` vs `pwsh`, or a setting) and **Copy as curl (redacted)**, plus **Copy raw** / **Copy raw (redacted)**; each calls the audited endpoint then writes to the clipboard. Keymap gains `copy-as-curl` (default `ctrl+shift+c`) and `copy-raw` (default `ctrl+shift+s`), both remappable.

- [ ] **Step 1: Write the failing e2e** `copy.spec.ts` (vite-preview, API mocked): the curl route is mocked to echo `redacted`/`shell` params; clicking **Copy as curl** calls `redacted=false`, **Copy as curl (redacted)** calls `redacted=true`; the clipboard (or a captured call) receives the mocked body. Add a `keymap.test.ts` assertion that `copy-as-curl` and `copy-raw` resolve to their default chords.
- [ ] **Step 2: Run** `npm --prefix ui run test` and the e2e → FAIL.
- [ ] **Step 3: Implement** the shared copy control + keymap entries + client calls. Captured bytes are never inserted as HTML (the §11.1 lint already bans it); copied text is set via the clipboard API, not the DOM.
- [ ] **Step 4: Run** `npm --prefix ui run lint`, `npm --prefix ui run test`, `npm --prefix ui run e2e` → green.
- [ ] **Step 5: FULL GATE** (plus the UI e2e; run the new spec under `--repeat-each=3` to catch a headless-Linux flake early, per the CI lesson).
- [ ] **Step 6: Commit** `feat(ui): copy-as-curl and copy-raw, redacted and plain`.

---

### Task 4: the redaction golden gate

**Files:**
- Modify: `crates/crosswire-server/tests/gates.rs` (new `redaction_golden` test, mirroring `audit_completeness` / `ca_key_never_leaves` which drive a real `serve`)
- Modify: `docs/security-gates.md` (gate #14 → the test path; correct the stale #3/#15/#16/#18 wording the whole-branch review flagged)

**Interfaces:**
- Consumes: the real `crosswire serve` harness used by `crosswire-server/tests/gates.rs`; the `/curl` and `/raw` routes (Task 2); the store's ingest path to seed exchanges.
- Produces: the DoD gate asserting redacted copies + logs are secret-free while unredacted copies are not.

- [ ] **Step 1: Write the failing test** `redaction_golden`:
  - Seed exchanges whose **headers** carry `Authorization: Bearer <unique-jwt>`, `Cookie: session=SENT_COOKIE`, `X-API-Key: SENT_KEY`, and whose **inline body** carries `access_token=SENT_AT&api_key=SENT_AK`. All sentinels are unique high-entropy strings.
  - Assert the **redacted** curl (both `sh` and `pwsh`) and **redacted** raw contain none of the five sentinels.
  - Assert the **unredacted** curl and raw **do** contain them.
  - Assert the captured process **stdout+stderr** for the run and `audit.jsonl` contain **none** of the five sentinels.
  - A quoting case: a header value with `'`, `$(id)`, a backtick, a newline and `;` round-trips through `quote` (Task 1) to exactly those bytes as one argument for both shells (structural, not executed).
- [ ] **Step 2: Run** `cargo test -p crosswire-server --test gates redaction_golden` → FAIL (before the earlier tasks it won't compile; this task is scheduled last, so it should fail only on the assertions if at all).
- [ ] **Step 3: Implement** the test and the `docs/security-gates.md` edits. If an assertion fails because a real secret leaks (e.g., a log line prints a header), fix the leak in the owning crate — that is the gate doing its job — and note it in the commit body.
- [ ] **Step 4: Run** `cargo test -p crosswire-server` → green.
- [ ] **Step 5: FULL GATE** (the golden is a CI gate, not maintainer-only — confirm it runs under the normal `just test`).
- [ ] **Step 6: Commit** `test(server): redaction golden over a seeded project`.

---

## Notes for the executor

- Task order is linear (0 → 1 → 2 → 3 → 4); each consumes the prior. Suggested models: Task 0 and Task 1 and Task 4 are security-correctness (opus); Task 2 and Task 3 are wiring (sonnet).
- The §9.4 audit set and the audit-completeness gate's asserted list gain `copy.curl` / `copy.raw` — update that list in Task 2 so the existing gate stays green.
- Do not change the redaction placeholder or default policy. Do not touch the preview layer (separate sub-plan).
