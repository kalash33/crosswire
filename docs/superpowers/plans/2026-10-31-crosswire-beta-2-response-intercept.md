# Beta-2: Response Intercept Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Hold, inspect, edit and forward (or drop) HTTP responses in intercept — by rule and on demand — with the same held list, editor and `Limits` as requests.

**Architecture:** The relay gains an async response hold mirroring the request path: `Handler::holds_response` (sync, from the heads) decides buffering; the body is read under `intercept_hold.min(capture_cap)` into budget-charged buffers (or, when too large, only the head is held and upstream is left unread for backpressure); `Handler::on_response` awaits the operator; `Forward`/`Modify`/`Drop` are applied and `Modify` records **response** diffs. `Intercept` keeps one queue of `Held` entries with a `kind`, new rules (MIME skips, hold-responses switch, response timeout policy) and a per-exchange flag set keyed by a relay-generated `ExchangeTag` that both hooks receive. The API and UI add the matching fields and controls.

**Tech Stack:** Rust (crosswire-proxy, crosswire-api, crosswire-h1 `apply_edits` with `ResponseContext`), Svelte 5 + TypeScript, Playwright, the existing proxy test rig and §18 flat-RSS harness.

**Spec:** docs/superpowers/specs/2026-10-31-crosswire-beta-2-response-intercept-design.md

## Global Constraints

- `unsafe_code = "forbid"`; MSRV 1.88; `cargo build` Node-free; `cargo deny` clean; **no new dependency**.
- Conventional Commits, ≤72-char subjects, **no attribution trailers** (verify `git log -1 | grep -c -i claude` == 0 after each commit).
- Additive wire changes only: new DTO fields default on the way in; the `Handler` trait change stays inside the workspace and `NoopHandler` keeps streaming.
- Defaults (verbatim from the spec): `hold_responses = false`; `response_on_timeout = Forward`; `skip_mime_types = ["image/", "font/", "audio/", "video/", "text/css", "text/javascript", "application/javascript"]`; hold limit `intercept_hold.min(capture_cap)`; `1xx`, `101`, tunnels and `CONNECT` never held; `Respond504` never produced for a response.
- FULL GATE before a task is done (`just lint`, `just doc`, `cargo deny check`, the crate's tests, UI lint/test/e2e where the UI changes). **`cargo test --workspace` runs once, in Task 3, started in the background** (dev-box ruling); Tasks 0–2 run their own crate's tests. **The committed UI bundle is rebuilt once, in Task 3.**
- Flat-RSS tests use `#[tokio::test(flavor = "multi_thread", worker_threads = 2)]` and a yielding drive (binding ruling from sub-plan 2b).
- Reuse: the request hold machinery (`relay.rs:360-470`), `CaptureBuf` + `Budget::try_reserve`, `apply_edits(.., Some(ResponseContext), ..)`, `changes_for`, the `tests/intercept.rs` rig, `HeldList`/`ByteEditor`/`RulesEditor`.

## Review Focus

- A response larger than `intercept_hold` must never be buffered past the limit and must still reach the client after Forward — Task 0 `response_over_hold_limit_is_not_editable_and_streams_after_forward` + `response_intercept_hold_flat_rss`.
- A `Drop` on a held response must close the client and record the exchange (never leave the client waiting or upstream half-read) — Task 0 `held_response_dropped_closes_client_and_records_error`.
- A held response must not stall the client forever: the timeout policy fires and `forward_all` releases responses — Task 0 `response_timeout_forwards_or_drops_per_rule`, `forward_all_releases_mixed_queue`.
- The flag holds exactly one response for exactly that request, even on a reused upstream connection carrying other exchanges — Task 0 `intercept_response_flag_holds_exactly_that_response`.
- `101`/`1xx` are never held and `Modify` on a non-editable response is refused with a stable error; response bytes shown in the intercept editor stay inert — Task 0 `upgrade_101_and_interim_1xx_never_held`, `modify_on_non_editable_response_is_refused`; Task 3 `xss-repeater-intercept` held-response case.

---

### Task 0: proxy — relay response hold + `Intercept` response support

**Files:**
- Modify: `crates/crosswire-proxy/src/handler.rs` (`ExchangeTag`, `HeldRequest.tag`, `HeldResponse`, `Handler::holds_response`/`on_response`, remove `on_response_head`, `NoopHandler`), `crates/crosswire-proxy/src/relay.rs` (tag per exchange; response hold around the final-head path `:590-660`; response diffs on the recorded response near `:733`), `crates/crosswire-proxy/src/intercept.rs` (`HeldKind`, `status`, rules fields, flag set, `forward` signature, `validate_response_head`, `skipped_by_mime`, `holds_response`/`on_response`), any other `Handler` impl in the crate's tests
- Test: `crates/crosswire-proxy/tests/intercept.rs` (new cases on the existing `rig`), `crates/crosswire-proxy/tests/limits_rss_intercept.rs` (response leg), unit tests in `intercept.rs`

**Interfaces:**
- Consumes: `Decision` (`#[non_exhaustive]`: `Forward | Drop | Modify { head, body, changes } | Respond504`), `CaptureBuf::new(cap, &budget)`, `Budget::try_reserve(n) -> Option<Reservation>`, `apply_edits(msg, edits, cfg, Some(ResponseContext{..}), ChangeReason::InterceptEdit)`, `Message::with_diffs(Vec<Diff>)`, `Diff::from(&Change)`.
- Produces:
  - `#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)] pub struct ExchangeTag(pub u64)`; the relay assigns one per exchange from a process-wide `AtomicU64` and passes it to both hooks.
  - `HeldRequest<'a>` += `pub tag: ExchangeTag`.
  - `pub struct HeldResponse<'a> { pub head: &'a RawMessage, pub req_head: &'a RawMessage, pub body: Option<&'a Bytes>, pub tag: ExchangeTag, pub scope_state: ScopeState, pub editable: bool }`.
  - `Handler`: `fn holds_response(&self, _resp: &RawMessage, _req: &RawMessage, _tag: ExchangeTag, _scope_state: ScopeState) -> bool { false }` and `fn on_response<'a>(&'a self, _resp: HeldResponse<'a>) -> BoxFuture<'a, Decision> { Box::pin(async { Decision::Forward }) }` (both defaulted); `on_response_head` is deleted.
  - `pub enum HeldKind { Request, Response }`; `HeldSummary` += `pub kind: HeldKind`, `pub status: Option<u16>`.
  - `pub enum ResponseOnTimeout { Forward, Drop }`; `InterceptRules` += `pub skip_mime_types: Vec<String>`, `pub hold_responses: bool`, `pub response_on_timeout: ResponseOnTimeout`; `Default` carries the spec's MIME list.
  - `Intercept::forward(&self, id: Ulid, edited: Option<EditedBytes>, intercept_response: bool) -> Result<(), InterceptError>`; `InterceptError::NotARequest` (flag on a response entry) and the existing `NotEditable` (Modify on a non-editable hold).
  - Relay semantics (spec §3): editable iff the final response has a `Content-Length` ≤ `intercept_hold.min(capture_cap)` (no chunked/close-delimited bodies are buffered) and is not `1xx`/`101`; a held non-editable response reads nothing from upstream until the decision; `Forward` writes head (+ buffered body) then streams; `Modify` writes the operator's bytes and sets the response diffs; `Drop` submits the draft with `resp_head` set, body cut, `ExchangeState::Error`, then `Next::Close { linger: false }`.

- [ ] **Step 1: Write the failing integration tests** in `tests/intercept.rs` using the existing `rig`/`get`/`resp` helpers (enable intercept and set `hold_responses: true` where needed): `held_response_is_listed_with_kind_and_status` (one held entry, `kind == Response`, `status == Some(200)`, `method/host/path` of the request); `held_response_forwarded_unchanged_reaches_client` (client receives the exact upstream bytes); `held_response_forwarded_with_edited_head_records_response_diffs` (edited status line `HTTP/1.1 418 Teapot` → client sees 418; the recorded exchange's **response** `diffs()` has one `Replace` with `ChangeReason::InterceptEdit`; the request has none); `held_response_dropped_closes_client_and_records_error` (client read returns EOF/reset; one exchange recorded with `state == Error` and `resp_head` present); `response_over_hold_limit_is_not_editable_and_streams_after_forward` (body = `intercept_hold + 1` bytes with a tiny hold cap set on the rig; `editable == false`; `forward(.., Some(edited), ..)` → `NotEditable`; plain forward → client receives all bytes); `response_timeout_forwards_or_drops_per_rule` (hold timeout 200 ms on the rig; `Forward` → client gets the response; `Drop` → connection closed); `hold_responses_off_and_no_flag_holds_nothing`; `intercept_response_flag_holds_exactly_that_response` (two sequential requests on one client keep-alive connection with upstream reuse; forward the first held request with `intercept_response: true` → its response is held, the second request's response is not; a third request's response is not held either); `mime_skip_and_out_of_scope_apply_to_responses` (`Content-Type: image/png` not held with default rules; `text/html` held; `skip_out_of_scope` + out-of-scope host not held); `forward_all_releases_mixed_queue`; `upgrade_101_and_interim_1xx_never_held` (a `100 Continue` then `200` holds only the 200; a `101` is relayed as today); `queue_cap_counts_both_kinds` (`intercept_queue = 2`, one request + one response held → a third hold applies the overflow policy); `modify_on_non_editable_response_is_refused`. Unit tests in `intercept.rs`: `validate_response_head_accepts_valid_and_rejects_garbage`, `skipped_by_mime_matches_prefix_case_and_params` (`IMAGE/PNG; charset=x` skipped by `image/`). `limits_rss_intercept.rs`: `response_intercept_hold_flat_rss` (a response body exactly at `intercept_hold` held, RSS flat during the hold; same sampler/assert shape as `intercept_hold_flat_rss`).
- [ ] **Step 2: Run** `cargo test -p crosswire-proxy` → the new tests FAIL (compile errors on the new API are expected — stub the signatures first if needed so the rest of the suite still runs).
- [ ] **Step 3: Implement** `handler.rs` (types + trait + NoopHandler), `intercept.rs` (kind/status/rules/flag set/`forward`/validators/`holds_response`/`on_response`; reuse `changes_for` with a response-head validator), `relay.rs` (tag; the hold in the final-response path; response diffs on the recorded response). Fix every other `Handler` impl the compiler finds.
- [ ] **Step 4: Run** `cargo test -p crosswire-proxy` → green, including the existing request suite byte-for-byte.
- [ ] **Step 5: FULL GATE** for the crate: `just lint`, `just doc`, `cargo deny check`, `cargo test -p crosswire-proxy -p crosswire-h1`.
- [ ] **Step 6: Commit** `feat(proxy): hold, edit and forward responses in intercept`.

---

### Task 1: API — kind, status, response rules, intercept-response flag

**Files:**
- Modify: `crates/crosswire-api/src/routes/intercept.rs` (`HeldDto`, `RulesDto`, `ForwardBody`, conversions, `forward_held`, error mapping, audit detail), `docs/api/openapi.json` (regenerated snapshot)
- Test: `crates/crosswire-api/tests/intercept.rs` (extend; follow how the existing tests seed held items)

**Interfaces:**
- Consumes: Task 0's `HeldKind`, `HeldSummary.{kind,status}`, `InterceptRules.{skip_mime_types,hold_responses,response_on_timeout}`, `ResponseOnTimeout`, `Intercept::forward(id, edited, intercept_response)`, `InterceptError::NotARequest`.
- Produces: `HeldDto` += `kind: String` (`"request"` | `"response"`), `status: Option<u16>`; `RulesDto` += `skip_mime_types: Vec<String>`, `hold_responses: bool`, `response_on_timeout: String` (`"forward"` | `"drop"`; any other value → 400 like `on_timeout`); `ForwardBody` += `intercept_response: Option<bool>`; `POST /held/{id}/forward` passes `intercept_response.unwrap_or(false)`; `NotARequest` → 400 with the stable error envelope; the `intercept.forward` / `intercept.drop` audit detail includes `kind`.

- [ ] **Step 1: Write the failing tests:** `rules_round_trip_includes_response_fields` (PUT then GET equals, incl. the default MIME list on a fresh serve); `rules_rejects_unknown_response_on_timeout` (400); `held_dto_carries_kind_and_status` (seed a held response the way the existing tests seed a held request, or through the proxy rig if that is how they do it; assert `kind == "response"`, `status == 200`); `forward_with_intercept_response_on_a_response_is_400`; `forward_passes_intercept_response_flag` (a held request forwarded with `{intercept_response: true}` → its response is then held); `audit_forward_and_drop_include_kind`.
- [ ] **Step 2: Run** `cargo test -p crosswire-api` → FAIL.
- [ ] **Step 3: Implement** the DTO fields, conversions, flag pass-through, error mapping and audit detail; regenerate the OpenAPI snapshot the way the repo's existing test/just recipe does.
- [ ] **Step 4: Run** `cargo test -p crosswire-api` → green (route-auth, OpenAPI and audit gates included).
- [ ] **Step 5: FULL GATE** for the crate: `just lint`, `just doc`, `cargo deny check`, `cargo test -p crosswire-api`.
- [ ] **Step 6: Commit** `feat(api): expose held kind, response rules and intercept-response`.

---

### Task 2: UI — held responses, rules fields, the per-request flag

**Files:**
- Modify: `ui/src/lib/api/types.ts` (regenerate or extend to match Task 1), `ui/src/lib/api/intercept.ts`, `ui/src/lib/stores/intercept.svelte.ts` (`forward(id, edited?, interceptResponse?)`), `ui/src/lib/components/HeldList.svelte`, `ui/src/routes/Intercept.svelte`, `ui/src/lib/components/RulesEditor.svelte`
- Test: `ui/e2e/intercept.spec.ts` (extend), `ui/src/lib/stores/intercept.test.ts` (extend if present, else add)

**Interfaces:**
- Consumes: Task 1's DTO fields.
- Produces: `HeldList` row shows `data-testid="held-kind"` (`REQ`/`RESP`) and, for responses, `data-testid="held-status"`; `Intercept.svelte`: `data-testid="intercept-response"` checkbox shown only for a focused request and sent with Forward (edited or not) as `intercept_response`; for a focused response the editor loads the response head/body via the existing `heldBytes`, Forward/Drop work, `to-repeater` is NOT rendered; `RulesEditor`: `rules-skip-mime` (comma list), `rules-hold-responses` (checkbox), `rules-response-timeout` (select forward/drop), round-tripped by the existing PUT. No keymap change.

- [ ] **Step 1: Write the failing e2e** (mocked API like the existing `intercept.spec.ts`): `held_response_row_shows_badge_and_status`; `held_response_edit_and_forward_posts_edited_head` (POST body has `head`, no `intercept_response`); `held_response_drop_posts_drop`; `to_repeater_hidden_for_a_held_response`; `forward_sends_intercept_response_flag` (check the box on a held request → POST body `{ intercept_response: true }`, unedited → no `head`/`body`); `rules_round_trip_includes_response_fields` (the PUT body carries the three new fields). Store unit test: `forward_passes_intercept_response`.
- [ ] **Step 2: Run** `npm --prefix ui run test` + `cd ui && npx playwright test intercept` → FAIL.
- [ ] **Step 3: Implement** the client/store/components.
- [ ] **Step 4: Run** `npm --prefix ui run lint`, `npm --prefix ui run test`, full `npm --prefix ui run e2e`, `npx playwright test intercept --repeat-each=3` → green; no new `eslint-disable`.
- [ ] **Step 5: FULL GATE** (minus the bundle rebuild / `ui-dist-check`, which Task 3 does): `just lint`, `just doc`, `cargo deny check`.
- [ ] **Step 6: Commit** `feat(ui): intercept held responses with rules and the request flag`.

---

### Task 3: gates — XSS over held responses, #18 ledger, bundle, full test

**Files:**
- Modify: `ui/e2e/xss-repeater-intercept.spec.ts` (held-response case), `docs/security-gates.md` (row 3 lists held responses; the `intercept_hold` #18 row cites `response_intercept_hold_flat_rss`), `crates/crosswire-server/assets/ui/**` (rebuilt bundle)

**Interfaces:**
- Consumes: Task 2's test ids; Task 0's `response_intercept_hold_flat_rss` name.

- [ ] **Step 1: Write the failing test** `held_response_payloads_render_as_text` in `xss-repeater-intercept.spec.ts`: mock a held response whose status line, headers and body carry every `XSS_CORPUS` payload; open it in the intercept view; assert the editor shows the bytes as text (`textContent` contains each payload), the badge/status render as text, and `liveProbe` is clean. Prove it exercises the surface by flipping one expected value → FAIL → restore.
- [ ] **Step 2: Update** `docs/security-gates.md` (row 3 and the #18 `intercept_hold` row) without overclaiming.
- [ ] **Step 3: Rebuild the bundle:** `just ui-build` then `just ui-dist-check` → clean; stage removals + additions.
- [ ] **Step 4: FULL GATE** — `just lint`, `just doc`, `cargo deny check`, `just ui-dist-check`, UI lint/test/full e2e + `--repeat-each=3` on `intercept` and `xss-repeater-intercept`, and `cargo test --workspace` (start it in the background first; report totals).
- [ ] **Step 5: Commit** `test: XSS and flat-RSS gates cover held responses`.

---

## Notes for the executor

- Order 0 → 1 → 2 → 3 (producer → consumer). Task 0 is the only hard one (relay concurrency, backpressure, budget): **opus** implementer and reviewer. Tasks 1–3: sonnet implementers; opus review for Task 3's gate, by diff size otherwise.
- Only Task 3 rebuilds the bundle and runs `cargo test --workspace` (in the background, once). Task 0/1 run their crate's tests; Task 2 the UI gate.
- The `Handler` trait change touches every impl in the workspace — the compiler lists them; fix them in Task 0, do not leave `on_response_head` behind.
- If a corpus payload executes in the intercept editor, that is a real §11.1 defect: STOP and report, do not relax anything.
