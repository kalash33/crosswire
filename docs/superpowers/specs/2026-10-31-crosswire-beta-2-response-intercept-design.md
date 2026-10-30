# Crosswire beta-2: response intercept — design spec

> **Status:** second of three beta sub-plans (§14 "beta — daily tools"); beta-1 (tags/notes +
> decoder) merged as PR #8. Authority: the product spec
> `docs/superpowers/specs/2026-10-22-crosswire-design.md` (§7.1 handler trait, §7.3 intercept,
> §11.6 `Limits`). Where this document and the product spec disagree, the product spec wins.

## 1. Goal

Let the operator **hold, inspect, edit and forward (or drop) HTTP responses** the way they
already can requests: by rule (responses of in-scope requests, minus MIME types that are never
interesting) and on demand ("intercept the response to this request"), inside the same held
list, with the same editor, under the same `Limits`.

## 2. What exists and what is missing

| Piece | Exists (requests) | Missing (responses) |
|---|---|---|
| Relay hook | `Handler::holds()` (sync, decides buffering) + `Handler::on_request(HeldRequest) -> BoxFuture<Decision>`; the relay buffers the body under `intercept_hold.min(capture_cap)` in two budget-charged copies, awaits the decision, honours `Forward`/`Modify`/`Drop`/`Respond504` (`relay.rs:360-470`) | `on_response_head` is **synchronous** and can only `Drop` (`relay.rs:596`); the head is written to the client at once and the body streams — **there is no response hold** |
| Handler impl | `Intercept`: `Held { id, host, method, path, size, editable, held_since, head, body, tx: oneshot<Decision> }`, `forward(id, edited)`, `drop(id)`, `forward_all()`, overflow + timeout policies, `bump()` for the UI | held **kind**, response status, MIME skip rules, a hold-responses switch, a response timeout policy, the per-request "intercept its response" flag |
| Edit engine | `apply_edits(msg, edits, cfg, ctx: Option<ResponseContext>, reason)` supports responses; `Message::with_diffs` exists on the model | the relay records diffs only on the request (`relay.rs:733`); `changes_for` validates a **request** head only |
| API | `/api/v1/intercept{,/enabled,/rules,/held/{id}/forward,/held/{id}/{part},/held/{id}/drop,/forward-all}`; `HeldDto { id, host, method, path, size, editable }`; `RulesDto { skip_extensions, skip_out_of_scope, overflow, on_timeout }`; `ForwardBody { head?, body? }` | `kind`, `status`, the new rule fields, the `intercept_response` flag |
| UI | `HeldList`, `Intercept.svelte` (forward / drop / forward-all / to-repeater, editable badge, Burp chords), `RulesEditor`, `ByteEditor` | kind badge + status, the "Intercept response" control, rule fields, hiding to-repeater for responses |

## 3. The relay hook (crosswire-proxy)

- `Handler` gains two methods, defaulting to "never hold" so `NoopHandler` and every existing
  handler keep streaming:
  - `fn holds_response(&self, resp: &RawMessage, req: &RawMessage, tag: ExchangeTag, scope_state: ScopeState) -> bool` — cheap and synchronous, decided from the response head, the request head and the per-exchange tag.
  - `fn on_response<'a>(&'a self, resp: HeldResponse<'a>) -> BoxFuture<'a, Decision>` — may await an operator decision.
  - `pub struct HeldResponse<'a> { pub head: &'a RawMessage, pub req_head: &'a RawMessage, pub body: Option<&'a Bytes>, pub tag: ExchangeTag, pub scope_state: ScopeState, pub editable: bool }`.
  - `pub struct ExchangeTag(u64)` — relay-generated per exchange (a process-wide counter), passed to **both** `on_request` (new field on `HeldRequest`) and the response hooks. It is the correlation key for "intercept the response to this request" and never leaves the proxy process.
- `on_response_head` is removed; its only use (`Drop`) moves into `on_response`.
- Relay behaviour when `holds_response` is true (after the final response head is parsed, before anything is written to the client):
  - **Editable** when the body is delimited and fits under `hold_limit = intercept_hold.min(capture_cap)`: the body is read fully into a budget-reserved buffer (the same two-copy scheme as requests: the capture and the forwarded copy), then `on_response` is awaited.
  - **Not editable** otherwise (larger, chunked beyond the limit, or `Content-Length` absent with close-delimiting): only the head is held; upstream is simply not read while held (TCP backpressure), and the body streams after the decision. `Modify` is refused for a non-editable hold (`InterceptError::NotEditable`).
  - `Forward` → head (and buffered body) written to the client, then the rest streams. `Modify { head, body, changes }` → the operator's bytes are written as given (the editor's assisted mode owns `Content-Length`), and `changes` are recorded as the **response** message's diffs (`Message::with_diffs`). `Drop` → both connections close; the exchange is recorded with the request, the response head as received, the body cut, and `ExchangeState::Error` (the same path as "client gone before the response head"; the backlog item on `ExchangeState::Error` semantics covers the naming). `Respond504` is never produced for a response.
  - Interim `1xx` responses, the `101` upgrade exchange (WebSocket), passthrough tunnels and `CONNECT` are **never** held.
  - A hold charges the in-flight budget exactly like a request hold; `intercept_queue` bounds the total held items (requests + responses together) with the existing overflow policy.

## 4. The handler (`Intercept`)

- `Held` gains `kind: HeldKind { Request, Response }` and `status: Option<u16>`; `HeldSummary` mirrors both. Response entries carry the originating request's `method`/`host`/`path`.
- `InterceptRules` gains `skip_mime_types: Vec<String>` (prefix match on the response `Content-Type` media type, case-insensitive; default `["image/", "font/", "audio/", "video/", "text/css", "text/javascript", "application/javascript"]`), `hold_responses: bool` (default **false** — Burp parity: responses are intercepted by rule only when switched on) and `response_on_timeout: ResponseOnTimeout { Forward, Drop }` (default `Forward`). The existing request fields are unchanged.
- `holds_response` is true when intercept is enabled **and** (the tag is in the "intercept my response" set **or** `hold_responses` is on) **and** the request's scope state is not `Out` when `skip_out_of_scope` is set **and** the response's media type is not skipped. The flag set is consulted and cleared in the same call, so a flagged request holds exactly one response.
- `forward(id, edited: Option<EditedBytes>, intercept_response: bool)` — for a **request** entry `intercept_response` inserts the entry's tag into the flag set; for a **response** entry it is rejected with `InterceptError::NotARequest` (the API maps it to 400). `changes_for` validates a response head with a new `validate_response_head` (status line + headers through the parser) when the entry is a response.
- Timeout: a held response times out after `intercept_hold_timeout` and acts per `response_on_timeout`. `forward_all` releases responses too. `held_bytes(id)` serves response head/body the same way.

## 5. API (crosswire-api, additive)

- `HeldDto` += `kind: "request" | "response"`, `status: Option<u16>`.
- `RulesDto` += `skip_mime_types: Vec<String>`, `hold_responses: bool`, `response_on_timeout: "forward" | "drop"`; `PUT /intercept/rules` accepts them with the same validation style as the existing fields.
- `ForwardBody` += `intercept_response: Option<bool>`; `POST /held/{id}/forward` passes it through; `true` on a response entry → 400 with the stable error envelope.
- Audit rows for `intercept.forward` / `intercept.drop` carry `kind`. OpenAPI snapshot regenerated. The route-auth gate's public set is unchanged (no new route).

## 6. UI

- `HeldList`: a kind badge (`REQ` / `RESP`) and, for responses, the status code; rows otherwise unchanged (method/host/path of the originating request).
- `Intercept.svelte`: for a focused **request**, an "Intercept response" checkbox (`data-testid="intercept-response"`) sent with Forward (edited or not); for a focused **response**, the editor shows the response head/body (editable when `editable`, with the same chips and the not-editable badge), Forward/Drop work, "Send to repeater" is hidden, and the existing Burp chords apply unchanged (no new keymap action).
- `RulesEditor`: a MIME-type list input, a "Hold responses" checkbox, a response-timeout select (forward / drop), in the existing style with the existing round-trip PUT.
- Rendering of every response value (status, head, body, badges) stays text / `ByteEditor` — §11.1 unchanged; the `xss-repeater-intercept` corpus spec gains a held-response case.

## 7. Testing

- **Proxy integration** (`crates/crosswire-proxy/tests/intercept.rs`, the existing rig): held response listed with kind/status; forwarded unchanged; forwarded with an edited head records **response** diffs and the client receives the edited bytes; dropped → client connection closes, exchange recorded with state `Error`; body over `intercept_hold` → held head only, not editable, body streams after Forward; timeout → forward and drop per `response_on_timeout`; `hold_responses` off + no flag → nothing held; flag on a request → exactly its response is held, a second response from the same connection is not; MIME skip (default list) and `skip_out_of_scope` apply; `forward_all` releases a mixed queue; a `101` upgrade and a `1xx` interim are never held; the queue cap counts both kinds. Unit: `validate_response_head`, `skipped_by_mime` (prefix, case, parameters like `; charset=`).
- **Flat-RSS** (`limits_rss_intercept.rs`): a response leg for the `intercept_hold` row (held response at the cap, RSS flat — the §13 #18 ledger row gains the test name).
- **API** (`crosswire-api/tests`): DTO fields round-trip; `intercept_response: true` on a response → 400; audit `kind`; the route-auth and OpenAPI gates still pass.
- **UI**: vitest for the store/client; `intercept.spec.ts` gains: held response appears with badge + status, edit + forward posts the edited head, drop, rules round-trip with the new fields, the intercept-response flag is sent on forward; `xss-repeater-intercept.spec.ts` gains a held-response case; `mouse-free` unchanged (chords unchanged).
- **Full gate** once (last task), including `cargo test --workspace` and the bundle rebuild.

## 8. Scope

**In:** §3–§7. **Out (YAGNI / later):** HTTP/2 per-stream holds (v0.4); WebSocket frame holds (v0.3); editing a non-editable (streamed) response; response-side match-and-replace (beta-3 consumes the same `Modify` path); rules persistence across `serve` restarts (pre-0.1.0 backlog item, unchanged); a "hold responses" status-bar toggle (the rules editor is enough for v0.1).

## 9. Global constraints (verbatim)

- `unsafe_code = "forbid"`; MSRV 1.88; `cargo build` Node-free; `cargo deny` clean; **no new dependency**.
- Conventional Commits, ≤72-char subjects, **no attribution trailers**.
- Additive wire changes only: new DTO fields are optional-with-defaults on the way in; the `Handler` trait change is within the workspace (the crate is not yet published) and `NoopHandler` keeps streaming.
- FULL GATE before a task is done; `cargo test --workspace` once per sub-plan in the last task (dev-box ruling), CI runs it on the PR.
- Reuse: the request hold machinery, `CaptureBuf`/budget reservations, `apply_edits` with `ResponseContext`, `HeldList`/`ByteEditor`/`RulesEditor`, the intercept test rig, the §18 flat-RSS harness.

## 10. Review focus (inputs the tests must cover)

- A response larger than `intercept_hold` must never be buffered past the limit (RSS flat) and must still reach the client after Forward.
- A `Drop` on a response must close the client connection and record the exchange, never leave the client waiting or the upstream half-read.
- A held response must not stall the client for ever: the timeout policy fires and `forward_all` works for responses.
- The "intercept response" flag must hold exactly one response for exactly that request (tag correlation), including on a reused upstream connection carrying other exchanges.
- `101`/`1xx`/tunnels are never held; `Modify` on a non-editable response is refused with a stable error; the §11.1 corpus holds for response bytes shown in the intercept editor.

## 11. Acceptance

- Responses can be held by rule and on demand, edited and forwarded or dropped, within `Limits`; the proxy integration suite above is green; the flat-RSS response leg is green on CI-Linux.
- The API exposes kind/status/rules/flag; the UI shows and edits held responses; the XSS gate covers them.
- Full CI gate green on ubuntu + macOS; no new dependency; `docs/security-gates.md` row 3 and the #18 ledger updated.
