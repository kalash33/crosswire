# Crosswire R1: schema freeze — design spec

> **Status:** first of four 0.1.0 release-tail sub-plans (R1 schema freeze → R2 release pipeline +
> governance → R3 client destination-gate ranges → R4 docs/launch → tag). Authority: the product
> spec `docs/superpowers/specs/2026-10-22-crosswire-design.md` §6.1 ("`Message` schema, frozen in
> v0.1"; evolution rule: additive-only, unknown fields ignored, `schema_version` in the API
> envelope, `user_version` in SQLite; the freeze takes effect at the 0.1.0 tag) and §12
> (project-format migrations with snapshot). Where this document and the product spec disagree,
> the product spec wins.

## 1. Goal

Make the recorded `Exchange`/`Message` wire format and the API envelope **frozen and
forward-compatible** before the 0.1.0 tag: every enum on the wire is a stable string code with an
`Unknown` fallback, every variant family is tagged the same way, optional fields default, the one
duplicated field is gone, the versions say so (`SCHEMA_VERSION = 2`, project `user_version = 6`),
the format is documented, and a committed golden fixture turns any future wire change into a
deliberate edit.

## 2. What exists and what changes

| Item (pre-0.1.0 backlog) | Today | Frozen |
|---|---|---|
| D1 core enums `ChangeOp`, `ChangeReason`, `EncodingHint` (crosswire-core `change.rs`, `hint.rs`) | derive-default PascalCase codes, no `Unknown` | `#[serde(rename_all = "kebab-case")]` + `#[serde(other)] Unknown` (`ChangeReason::IntruderPosition(u16)` stays externally tagged: `{"intruder-position": 3}`) |
| D1 `Severity`, `DiagnosticCode` (crosswire-core `diagnostic.rs`) | `Unknown` present, PascalCase on the wire while `as_str()` and the corpus `.expect.json` files use snake_case (`duplicate_content_length`) | `#[serde(rename_all = "snake_case")]` so the wire code **equals `as_str()`** — the corpus is the authority for diagnostic codes; a round-trip test pins `serde == as_str` for every variant of `DiagnosticCode::ALL` |
| D2 `StartLineSpans`, `FramingRecord` (model `message.rs:23`, `:85`) | externally tagged kebab-case | `#[serde(tag = "type", rename_all = "kebab-case")]` like `Head`/`Body`/`ChainResult`/`Event` |
| D3 `Head`, `Body`, `ChainResult`, `Event`, `ExchangeState`, `ScopeState`, `NamespaceKind` | tagged/kebab but no `Unknown` (`Fidelity`, `MarkerKind`, `TunnelReason` already have one) | each gains `#[serde(other)] Unknown` (a unit variant; for `type`-tagged enums the unknown `type` string maps to it) |
| D6 defaults | `Message.{framing,body,…}` default; `Http1Raw.parser_version` and `Event::Received.seq` do not | `#[serde(default = "…")]` on `parser_version` (→ `HTTP1_PARSER_VERSION`) and `seq` (→ 0); `Tunnel` counters default to 0 |
| D-c `ConnectionInfo.cert_override: bool` (model `message.rs:474`) | stored beside `upstream_chain_result: ChainResult { NotTls, Valid, Invalid, Overridden }` — a duplicate | field **removed from the struct and the wire**; `cert_override()` becomes `matches!(upstream_chain_result, ChainResult::Overridden)`; `with_cert_override` removed and its callers (client `transport.rs:273`, proxy `relay.rs:1352`) set the chain result instead; the repeater API/tools DTO field `cert_override` is **kept** as a derived value (it is a product feature — the §7.2 badge) |
| D-a `ExchangeState::Error` | undocumented meaning | documented contract: "the exchange did not complete normally (relay failure, client gone, operator drop); the recorded heads/bodies are whatever was received" — no new field |
| D-b `Marker.detail` | free text | documented as human-readable, non-contractual; `MarkerKind` stays the contract |
| D-d `TunnelReason` | already a dedicated enum | no change; the `Capture` Rust type is not on the wire — not renamed |
| D7 reserved names | — | `docs/format.md` reserves `truncation`, `error_detail`, `interim` as future top-level `Message` keys (readers ignore unknown keys today; the reservation is documentation) |
| Versions | `SCHEMA_VERSION = 1` never emitted; project `USER_VERSION = 5` | `SCHEMA_VERSION = 2`; `USER_VERSION = 6`; both emitted (see §4, §5) |

## 3. Forward compatibility (the evolution rule, made testable)

- **Unknown codes never fail a read.** Every wire enum deserializes an unrecognised string code (or unknown `type` tag) to its `Unknown` variant; every struct ignores unknown keys (`crates/crosswire-model/tests/unknown_fields.rs` already proves the key half — it gains the code half for every enum).
- **The freeze guard:** `crates/crosswire-model/tests/fixtures/exchange.v2.json` — one complete `Exchange` exercising every enum family (an `http1-raw` request with diagnostics of each severity, a response with a `Modify` diff carrying `intercept-edit`, a `blob` body with `truncated_at`, a `tunnel` message, `scope_state`, `namespace`, `state`) — must deserialize and re-serialize **byte-identically** (`serde_json` with sorted keys via `serde_json::to_string` of a `BTreeMap`-backed `Value` round trip, documented in the test). Changing the wire means editing this file on purpose.
- **Pre-release projects are not migrated (ruling).** Projects with `user_version < 6` were written by pre-release builds whose enum encoding differs; no public users exist. `Store::open` refuses them with a dedicated `StoreError::PreReleaseProject { version }` whose message says "created by a pre-release build of Crosswire; re-record it with 0.1.0" — never `Corrupt`. `migrate()` keeps the 1→5 steps for the record but `open` short-circuits to the refusal for any `version < 6`; a fresh project is created at 6.

## 4. API envelope (crosswire-api)

- Every response that passes through the auth gate (`auth.rs::gate`), **including the 401/421/404 the gate itself produces**, carries the header `Crosswire-Schema-Version: 2`.
- `GET /api/v1/health` returns `{ "status": "ok", "schema_version": 2, "format_version": 6 }` (`HealthResponse` gains the two `u32` fields; OpenAPI snapshot regenerated).
- No other route changes. `docs/api.md` gains an "Envelope and versions" paragraph.

## 5. Project format document

- `docs/format.md` becomes "version 6" and gains a **"Message wire format"** section: the JSON shape of `Exchange`/`Message` as stored in `exchanges.json`, every enum with its code list and tagging style, the `Unknown` rule, the defaults, the reserved names, and the `schema_version`/`user_version` pairing (2 ↔ 6). The migrations list notes that 6 has no data migration and that pre-6 projects are refused.
- `CHANGELOG.md` `[Unreleased]` records the freeze and the pre-release break in plain words.

## 6. Scope

**In:** §2–§5, the UI's hand-written `types.ts` mirror updated for the health fields (type-only), the fixtures/tests that encode PascalCase codes today (the grep found only `crosswire-proxy/src/listener.rs` — verify and fix), `docs/security-gates.md` unchanged.
**Out:** a JSON-Schema export (`schemars` would be a new dependency — the documented format + OpenAPI are the published schema for v0.1); migrating pre-release projects; renaming the `Capture` Rust type; any API route beyond the header and the health fields; the HTTP/2 frame schema (v0.2a, already shaped in §6.1).

## 7. Global constraints (verbatim)

- `unsafe_code = "forbid"`; MSRV 1.88; `cargo build` Node-free; `cargo deny` clean; **no new dependency**.
- Conventional Commits, ≤72-char subjects, **no attribution trailers**.
- The wire change is deliberately breaking for pre-release projects (ruling above) and additive for everything else; Rust API changes stay inside the workspace (crates are `publish = false`).
- FULL GATE before a task is done; `cargo test --workspace` once in the last task (dev-box ruling); the committed UI bundle is untouched (type-only UI change) and `just ui-dist-check` proves it.
- Reuse: `unknown_fields.rs`, the corpus `.expect.json` convention (`as_str` codes), `refuse_hostile_schema()`'s error style in `project.rs`, the OpenAPI snapshot test (`UPDATE_OPENAPI=1`).

## 8. Review focus (inputs the tests must cover)

- An unknown code string or unknown `type` tag in ANY wire enum deserializes to `Unknown` and never errors; a known code round-trips to the same variant.
- A pre-6 project opens to `PreReleaseProject` (not `Corrupt`, not a panic, not a silent migration); a version-7 project still reports "too new".
- Dropping `cert_override` changes no behaviour: on the override path the client/proxy set `ChainResult::Overridden`, and the repeater DTO's `cert_override` is `true` exactly then.
- The golden fixture round-trips byte-identically, and a deliberately altered copy fails the test.
- `Crosswire-Schema-Version: 2` is on a 200, on a gate-produced 401 and on a 421; `health` carries 2/6.

## 9. Acceptance

- All enums in §2 carry the frozen codes + `Unknown`; defaults and tagging as specified; `cert_override` gone from the wire with equivalence proven.
- `SCHEMA_VERSION = 2`, `USER_VERSION = 6`, pre-6 refusal, header + health fields, OpenAPI snapshot and `types.ts` updated.
- `docs/format.md` v6 with the Message section; `docs/api.md` envelope paragraph; CHANGELOG entry.
- Full CI gate green on ubuntu + macOS; corpus gate green (codes unchanged in meaning); no new dependency.
