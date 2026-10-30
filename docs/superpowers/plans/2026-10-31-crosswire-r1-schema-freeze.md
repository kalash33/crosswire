# R1: Schema Freeze Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Freeze the recorded `Exchange`/`Message` wire format and the API envelope before the 0.1.0 tag: stable string codes with `Unknown` fallbacks everywhere, uniform tagging, defaults, the duplicate `cert_override` gone, `SCHEMA_VERSION = 2` / `user_version = 6` emitted, the format documented, and a golden fixture that makes any future wire change deliberate.

**Architecture:** Serde attributes on the core/model enums do the freezing (kebab-case for the model and core change/hint enums, snake_case for diagnostics so the wire equals `as_str()` and the corpus); `#[serde(other)] Unknown` and `#[serde(default)]` make reads forward-compatible; a committed golden JSON fixture plus an unknown-code test suite guard it. The store bumps `USER_VERSION` to 6 and refuses pre-6 projects with a dedicated error instead of migrating them (ruling: no public users). The API stamps `Crosswire-Schema-Version: 2` on every gated response and reports both versions in `health`.

**Tech Stack:** Rust (serde, serde_json), SQLite `user_version`, axum middleware, utoipa snapshot; Markdown docs.

**Spec:** docs/superpowers/specs/2026-10-31-crosswire-r1-schema-freeze-design.md

## Global Constraints

- `unsafe_code = "forbid"`; MSRV 1.88; `cargo build` Node-free; `cargo deny` clean; **no new dependency** (no `schemars`).
- Conventional Commits, ≤72-char subjects, **no attribution trailers** (verify `git log -1 | grep -c -i claude` == 0 after each commit).
- Codes (verbatim): model enums and `ChangeOp`/`ChangeReason`/`EncodingHint` → `kebab-case`; `Severity`/`DiagnosticCode` → `snake_case` equal to `as_str()`; every wire enum has `#[serde(other)] Unknown`; `type`-tagged families: `Head`, `Body`, `ChainResult`, `Event`, `StartLineSpans`, `FramingRecord`.
- Versions: `SCHEMA_VERSION = 2`; `USER_VERSION = 6`; pre-6 projects → `StoreError::PreReleaseProject { version }` (never `Corrupt`); >6 → the existing "too new" error.
- FULL GATE per task for its crates; `cargo test --workspace` once, in Task 2, started in the background (dev-box ruling); the UI bundle is NOT rebuilt (type-only UI change) — `just ui-dist-check` proves it unchanged.
- Reuse: `crates/crosswire-model/tests/unknown_fields.rs`, the corpus `.expect.json` convention, `project.rs` `refuse_hostile_schema()` error style, the OpenAPI snapshot test (`UPDATE_OPENAPI=1 cargo test -p crosswire-api --test openapi`).

## Review Focus

- An unknown code or unknown `type` tag in ANY wire enum reads as `Unknown`, never an error — Task 0 `every_wire_enum_reads_unknown_codes`.
- A pre-6 project opens to `PreReleaseProject` (not `Corrupt`, not a panic, not a silent migration); a version-7 project still reports too-new — Task 1 `open_refuses_pre_release_project`, `open_still_rejects_newer_project`.
- Dropping `cert_override` changes no behaviour: the override path sets `ChainResult::Overridden`, and the repeater DTO's `cert_override` is `true` exactly then — Task 0 `cert_override_is_derived_from_chain_result` (model) + the existing repeater/client override tests kept green.
- The golden fixture round-trips byte-identically and an altered copy fails — Task 0 `frozen_exchange_fixture_round_trips_byte_identical`, `altered_fixture_is_detected`.
- `Crosswire-Schema-Version: 2` is on a 200, on a gate-produced 401 and on a 421; `health` carries 2/6 — Task 2 `schema_version_header_on_every_gated_response`, `health_reports_schema_and_format_versions`.

---

### Task 0: core + model wire freeze

**Files:**
- Modify: `crates/crosswire-core/src/change.rs` (`ChangeOp`, `ChangeReason`), `crates/crosswire-core/src/diagnostic.rs` (`Severity`, `DiagnosticCode`), `crates/crosswire-core/src/hint.rs` (`EncodingHint`), `crates/crosswire-model/src/message.rs` (`StartLineSpans:23`, `FramingRecord:85`, `Head:258`, `Body:307`, `ChainResult:440`, `ConnectionInfo:464-478`, `parser_version:268`, tunnel counters), `crates/crosswire-model/src/exchange.rs` (`ScopeState:28`, `NamespaceKind:44`, `ExchangeState:87`), `crates/crosswire-model/src/event.rs` (`Event:16`, `seq`), `crates/crosswire-model/src/lib.rs:54` (`SCHEMA_VERSION`), `crates/crosswire-client/src/transport.rs:273`, `crates/crosswire-proxy/src/relay.rs:1352`, `crates/crosswire-api/src/routes/repeater.rs:82-110`, `crates/crosswire-tools/src/repeater.rs:84-286`, `crates/crosswire-proxy/src/listener.rs` (the one PascalCase literal the grep found — verify)
- Create: `crates/crosswire-model/tests/fixtures/exchange.v2.json`, `crates/crosswire-model/tests/freeze.rs`
- Test: `crates/crosswire-model/tests/unknown_fields.rs` (extend), `crates/crosswire-core` unit tests

**Interfaces:**
- Consumes: nothing from this plan.
- Produces:
  - Core: `#[serde(rename_all = "kebab-case")]` + `#[serde(other)] Unknown` on `ChangeOp`, `ChangeReason`, `EncodingHint`; `#[serde(rename_all = "snake_case")]` on `Severity` and `DiagnosticCode` (both already have `Unknown` — keep it `#[serde(other)]`); `DiagnosticCode::as_str()` unchanged and now equal to the wire code.
  - Model: `StartLineSpans`, `FramingRecord` → `#[serde(tag = "type", rename_all = "kebab-case")]`; `Unknown` unit variant with `#[serde(other)]` on `Head`, `Body`, `ChainResult`, `Event`, `ExchangeState`, `ScopeState`, `NamespaceKind`; `#[serde(default = "default_parser_version")]` on `Http1Raw.parser_version` (returns `HTTP1_PARSER_VERSION`), `#[serde(default)]` on every `Event` `seq` and on the `tunnel` `bytes_in`/`bytes_out`/`duration_ms`; `ConnectionInfo` loses the `cert_override` field and `with_cert_override`; `pub fn cert_override(&self) -> bool { matches!(self.upstream_chain_result, ChainResult::Overridden) }` stays as the derived getter; `pub const SCHEMA_VERSION: u32 = 2`.
  - Callers: `transport.rs:273` and `relay.rs:1352` stop calling `with_cert_override` and instead ensure the chain result is `Overridden` on the override path (read the surrounding code: where `chain_error.is_some()` was the signal, set/keep `ChainResult::Overridden` there); the repeater API/tools `cert_override: bool` DTO fields are filled from the getter.
  - Fixture `exchange.v2.json`: one `Exchange` covering every family — `http1-raw` request head with `spans` and `parser_version`, diagnostics with `info`/`warning`/`error` severities and three distinct codes, a response with a `replace` diff of reason `intercept-edit`, a `blob` body with `truncated_at` and `capture_dropped`, a second `tunnel`-kind message (`reason: ws`), `framing` with a `type`-tagged record, `connection` with `upstream_chain_result: {"type":"overridden"}`, `scope_state: in`, `namespace: {kind: history}`, `state: committed`, `tags`, `notes`, `timing`.

- [ ] **Step 1: Write the failing tests.** `tests/freeze.rs`: `frozen_exchange_fixture_round_trips_byte_identical` (read the fixture → `serde_json::from_str::<Exchange>` → `serde_json::to_value` → compare to `serde_json::from_str::<Value>(fixture)` for equality AND `to_string` of both `Value`s equal — `Value` keeps map order sorted, so this is byte-stable); `altered_fixture_is_detected` (flip one code string in a copy, e.g. `"replace"` → `"replaced"`, assert the diff `op` deserializes to `ChangeOp::Unknown` and the round trip is NOT identical); `every_wire_enum_reads_unknown_codes` (for each of the 13 enums, deserialize `"no-such-code"` (or `{"type":"no-such-type"}` for tagged ones) → `Unknown`, and deserialize each known code → the expected variant); `diagnostic_wire_codes_equal_as_str` (for every `DiagnosticCode::ALL` and every `Severity`: `serde_json::to_string(&c) == format!("\"{}\"", c.as_str())`); `cert_override_is_derived_from_chain_result` (`ConnectionInfo` with `Overridden` → `true`; `Valid`/`Invalid`/`NotTls` → `false`; the serialized JSON has no `cert_override` key). Extend `unknown_fields.rs` to assert `parser_version` and `seq` default when absent.
- [ ] **Step 2: Run** `cargo test -p crosswire-model -p crosswire-core` → FAIL.
- [ ] **Step 3: Implement** the attributes, variants, defaults, the `cert_override` removal and its four callers, `SCHEMA_VERSION = 2`; write the fixture by hand from the spec's shape (then let the test's `to_string` tell you the canonical form and commit THAT form so the round trip is identical).
- [ ] **Step 4: Run** `cargo test -p crosswire-model -p crosswire-core -p crosswire-client -p crosswire-proxy -p crosswire-tools -p crosswire-api` → green; the corpus gate (`every_diagnostic_code_has_a_corpus_case`) and the h1 golden suite still pass (codes unchanged in meaning).
- [ ] **Step 5: FULL GATE** for the crates: `just lint`, `just doc`, `cargo deny check`.
- [ ] **Step 6: Commit** `feat(model): freeze wire codes, tagging and defaults for 0.1.0`.

---

### Task 1: store format 6 + pre-release refusal + `docs/format.md`

**Files:**
- Modify: `crates/crosswire-store/src/schema.rs:9` (`USER_VERSION = 6`; a 5→6 step that only stamps the version — no data migration — with a comment that pre-6 projects are refused before reaching it), `crates/crosswire-store/src/project.rs:116-135` (the refusal), `crates/crosswire-store/src/error.rs` (`StoreError::PreReleaseProject { version: u32 }`), `docs/format.md` (version 6; new "Message wire format" section; migrations list)
- Test: `crates/crosswire-store/tests/` (the project-open test file that covers `NotAProject`/too-new today)

**Interfaces:**
- Consumes: Task 0's frozen codes (documented, not imported).
- Produces: `StoreError::PreReleaseProject { version: u32 }` with `Display` "project was created by a pre-release build of Crosswire (format {version}); re-record it with 0.1.0"; `Store::open` order: `user_version == 0` → `NotAProject`; `> USER_VERSION` → the existing too-new error; `1..=5` → `PreReleaseProject`; `6` → open. A fresh project is created at 6. `docs/format.md`: title "version 6"; the "Message wire format" section lists, per enum, the code strings and tagging (`type`-tagged families named), the `Unknown` rule, the defaults, the reserved top-level `Message` keys `truncation`, `error_detail`, `interim`, and the pairing `schema_version 2 ↔ user_version 6`; the migrations list notes 6 = version stamp only, pre-6 refused.

- [ ] **Step 1: Write the failing tests:** `open_refuses_pre_release_project` (create a project, set `PRAGMA user_version = 5`, reopen → `Err(PreReleaseProject { version: 5 })`, and the same for 1); `open_still_rejects_newer_project` (set 7 → the existing too-new variant); `fresh_project_is_format_6` (`PRAGMA user_version` == 6 after create).
- [ ] **Step 2: Run** `cargo test -p crosswire-store` → FAIL.
- [ ] **Step 3: Implement** the error variant, the refusal in `open`, the version bump; write the `format.md` section from the spec's table (every code string spelled out — this document IS the published schema for v0.1).
- [ ] **Step 4: Run** `cargo test -p crosswire-store` → green (incl. the hostile-schema and kill-9 suites).
- [ ] **Step 5: FULL GATE** for the crate: `just lint`, `just doc`, `cargo deny check`.
- [ ] **Step 6: Commit** `feat(store): project format 6; refuse pre-release projects`.

---

### Task 2: API envelope, docs, changelog, full gate

**Files:**
- Modify: `crates/crosswire-api/src/auth.rs:331-383` (`gate`: add the header to every response it returns, including its own 401/421/404), `crates/crosswire-api/src/routes/misc.rs:19` (`HealthResponse` += `schema_version: u32`, `format_version: u32`), `docs/api/openapi.json` (regenerated), `ui/src/lib/api/types.ts` (`HealthResponse` mirror), `docs/api.md` ("Envelope and versions" paragraph near "Two sockets"/"Failures"), `CHANGELOG.md` (`[Unreleased]`: the freeze, the header, format 6, the pre-release break)
- Test: `crates/crosswire-api/tests/gates.rs` or a new `tests/envelope.rs`

**Interfaces:**
- Consumes: `SCHEMA_VERSION` (Task 0), `crosswire_store::USER_VERSION` or a `pub const FORMAT_VERSION` re-export (Task 1 — if `USER_VERSION` is `pub(crate)`, expose `pub const FORMAT_VERSION: u32 = USER_VERSION` in `crosswire-store`'s lib).
- Produces: header `Crosswire-Schema-Version: 2` on every response that passes through `gate` (wrap `next.run(req).await` results and the early returns alike); `HealthResponse { status: "ok", schema_version: 2, format_version: 6 }`.

- [ ] **Step 1: Write the failing tests:** `schema_version_header_on_every_gated_response` (a 200 from `/api/v1/health`, a 401 from a credential-less protected route, a 421 from a non-loopback `Host` — all carry `crosswire-schema-version: 2`); `health_reports_schema_and_format_versions` (JSON has `schema_version == 2`, `format_version == 6`); the OpenAPI snapshot test fails until regenerated.
- [ ] **Step 2: Run** `cargo test -p crosswire-api` → FAIL.
- [ ] **Step 3: Implement** the header, the health fields, regenerate the snapshot (`UPDATE_OPENAPI=1 cargo test -p crosswire-api --test openapi`), update `types.ts`, write the `api.md` paragraph and the CHANGELOG entry.
- [ ] **Step 4: Run** `cargo test -p crosswire-api` → green (route-auth, OpenAPI, audit gates included).
- [ ] **Step 5: FULL GATE — all of it:** `just lint`, `just doc`, `cargo deny check`, `just ui-dist-check` (must report NO drift — the UI change is type-only), `npm --prefix ui run lint`, `npm --prefix ui run test`, full `npm --prefix ui run e2e`, and `cargo test --workspace` started in the background first (report totals; the h1 golden/corpus/kill-9/flat-RSS suites all run here).
- [ ] **Step 6: Commit** `feat(api): schema and format versions in the envelope`.

---

## Notes for the executor

- Order 0 → 1 → 2. Task 0 is the risky one (13 enums, a removed field with four callers, the fixture) → **opus** implementer and reviewer; Tasks 1–2 sonnet, opus review for Task 2's full gate.
- If Task 0 finds a committed fixture or test that depends on PascalCase codes beyond `listener.rs`, fix it in Task 0 and list it in the report — do not weaken the codes.
- The fixture's canonical form: run the round-trip test once with the hand-written file, take the serializer's output as the committed file, re-run — identical. Never "fix" the test to tolerate reordering.
- A future wire change = a deliberate edit of `exchange.v2.json` + a `SCHEMA_VERSION` bump + a `format.md` entry; the test exists to make that visible in review.
