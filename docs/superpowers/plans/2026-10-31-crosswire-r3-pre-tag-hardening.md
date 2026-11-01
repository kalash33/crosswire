# R3 Pre-Tag Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the destination-gate SSRF gaps, give format-6 projects planner statistics, make the repeater API emit frozen codes, fix the browser-owned default shortcut and the five UI inconsistencies before `v0.1.0`.

**Architecture:** The gate canonicalises IPv6 transition forms to their inner IPv4 before applying class rules split into non-allowlistable and allowlistable sets, with an own-port rule that probes local ownership through a throwaway UDP bind. The store runs `ANALYZE`/`PRAGMA optimize` on clean close under a threshold. The API serialises change ops/reasons through the core types' serde. The UI derives every shortcut hint from the keymap and reads the new `enforce_strict` field.

**Tech Stack:** Rust (std only, no new deps), rusqlite, axum/utoipa, Svelte 5, Playwright.

**Spec:** `docs/superpowers/specs/2026-10-31-crosswire-r3-pre-tag-hardening-design.md`

## Global Constraints
No new dependency (no `Cargo.toml`/`Cargo.lock`/`package*.json` change); `unsafe_code = "forbid"`; `missing_docs = deny`; MSRV 1.88; §11.1 UI rules untouched; UI bundle rebuilt and `just ui-dist-check` clean; Conventional Commits ≤72-char subjects with real bodies; NO AI attribution trailer; full CI gate before push (`just lint`, `just doc`, `cargo deny check`, `just features`, `cargo test --workspace` once in the background, UI lint/check/test/e2e).

## Review Focus
1. A name that resolves to both a public IP and `169.254.169.254` → refused (every address is checked; Task 0 test).
2. `--allow 127.0.0.1:<api port>` → `OwnListener` refusal (Task 0 test, both gates).
3. A LAN IP of the host itself at the proxy port (`192.168.x.y:8080`) → `OwnListener` via the UDP probe (Task 0 test uses a bound loopback alias? no: the test binds a UDP socket to `127.0.0.1` to prove the probe primitive, and uses the gate on the machine's first non-loopback address from `UdpSocket::connect(8.8.8.8:53).local_addr()` if available, skipping otherwise with a note).
4. A project closed cleanly with 150 rows and threshold 100 → `sqlite_stat1` has an `exchanges` row and the size query uses `idx_exchanges_resp_len` (Task 1 test).
5. The repeater result for an edit with an `IntruderPosition(3)` reason → `{"op":"replace","reason":{"intruder-position":3}}` (Task 2 test).

---

### Task 0: destination gate (opus)
**Files:** Modify `crates/crosswire-client/src/gate.rs`, `src/error.rs` (new variants' Display), `src/transport.rs` (name denylist before resolution; `Unresolved` instead of the loopback mislabel), `crates/crosswire-server/src/serve.rs` (bind API listener first; both ports to both gates); Test `crates/crosswire-client/tests/gate.rs` (+ `tests/gate_transition.rs` for canonicalisation vectors); Docs `docs/security-gates.md:22` row 7, `crates/crosswire-client/README.md` gate section.
**Interfaces → Produces:** `GateRefusal::{Loopback, LinkLocal, Metadata, MetadataName(String), OwnListener(u16), Unresolved, Multicast, Broadcast, Unspecified}`; `pub fn canonical(ip: IpAddr) -> IpAddr` (public for tests); `DestinationGate::new(own_ports: Vec<u16>)` unchanged signature; `check(host, port, resolved, allowlisted) -> Result<(), GateRefusal>` order per spec D2.
- [ ] Step 1: failing tests: `canonical_unwraps_transition_forms` (`::ffff:7f00:1`→127.0.0.1, `::7f00:1`→127.0.0.1, `64:ff9b::a9fe:a9fe`→169.254.169.254, `2002:7f00:0001::`→127.0.0.1, Teredo `2001:0:4136:e378:8000:63bf:3fff:fdd2`→192.0.2.45 per RFC 4380 example, `::1` and `::` unchanged); `refuses_new_ranges_in_every_spelling` (0.1.2.3, 224.0.0.1, 239.255.255.250, 255.255.255.255, ff02::1, fd20:ce::254, `::ffff:e000:1`, `64:ff9b::ffff:ffff`); `allowlist_cannot_exempt_metadata_multicast_or_own_ports`; `allowlisted_name_resolving_to_metadata_is_refused`; `metadata_host_names_are_refused_before_resolution`; `empty_resolution_is_unresolved`; `private_and_cgnat_and_ula_are_allowed`; `own_port_on_local_interface_is_refused` (probe primitive + skip note); existing tests still pass.
- [ ] Step 2: implement per D1–D3; `serve.rs` bind order + both gates get both ports; update `docs/security-gates.md` row 7 text + test list and the client README.
- [ ] Step 3: `cargo test -p crosswire-client -p crosswire-server --test cli --test gates` (sequential), `just lint`, `cargo deny check`. Commit `fix(client): canonicalise transition forms and close destination-gate gaps`.

### Task 1: planner statistics on clean close (sonnet)
**Files:** Modify `crates/crosswire-store/src/writer.rs` (finish: ANALYZE/optimize per D4), `src/store.rs` or options struct (`analyze_threshold`, default 1000), `src/schema.rs` (doc note), `docs/format.md` (one sentence under maintenance/close); Test `crates/crosswire-store/tests/indexes.rs` (drop manual `analyse()`; new `clean_close_gathers_statistics_above_threshold` and `clean_close_runs_optimize_below_threshold` checking `sqlite_stat1`), module doc rewrite.
- [ ] Step 1: failing tests (threshold 100: 150 rows → `sqlite_stat1` has `exchanges`; 50 rows → no `exchanges` row but close succeeds; the size-query plan uses `idx_exchanges_resp_len` after a clean close without manual ANALYZE).
- [ ] Step 2: implement; keep kill-9 semantics untouched (Abandon path runs nothing).
- [ ] Step 3: `cargo test -p crosswire-store`, `just lint`, `cargo deny check`. Commit `feat(store): gather planner statistics on clean close`.

### Task 2: frozen codes in the repeater API + scope strict read (sonnet)
**Files:** Modify `crates/crosswire-api/src/routes/repeater.rs` (`ChangeDto {op: String, reason: serde_json::Value, start, end}` + `ToSchema`; `SendOutcomeDto` response schema), `src/routes/scope.rs` (`enforce_strict: bool` on the GET DTO), `docs/api/openapi.json` (regenerate), `docs/api.md`, `CHANGELOG.md`; Test `crates/crosswire-api/tests/repeater.rs` (assert `op == "replace"` and `reason` string/object forms), `tests/scope.rs` (GET returns `enforce_strict` matching the PUT).
- [ ] Step 1: failing tests; Step 2: implement with `serde_json::to_value(c.op())`/`to_value(c.reason())`; Step 3: `UPDATE_OPENAPI=1 cargo test -p crosswire-api --test openapi`, then `cargo test -p crosswire-api`, `just lint`, `cargo deny check`. Commit `feat(api): frozen change codes in repeater results; scope strict flag on GET`.

### Task 3: UI follow-through (sonnet)
**Files:** Modify `ui/src/lib/keymap.ts` (`toggle-intercept: ctrl+i`), `keymap.test.ts`, hard-coded hints → `chordFor` in `RepeaterResponse.svelte`, `ExchangeDetail.svelte`, `RepeaterTab.svelte`, `Intercept.svelte`; `StatusBar.svelte` (drop Intruder); `Scope.svelte` + `stores/scope.svelte.ts` + `api/types.ts` (read `enforce_strict`, remove the unknown-state hint path); `DecoderPane.svelte`/`routes/Decoder.svelte` (no Back, Esc no-op when embedded); `Intercept.svelte` (`showCopyRaw={false}`, Copy raw in toolbar); `RepeaterResponse.svelte` (use `HexWindow`); `api/types.ts` + `editor/assisted.ts` + `assisted.test.ts` (ChangeDto shape); e2e: `mouse-free.spec.ts`/`shell.spec.ts` press `Control+i`, `settings.spec.ts` asserts `Ctrl+I`, `xss-corpus.spec.ts` + Hex tab and diagnostics-group cases, scope spec asserts the hint is gone when the server reports the flag; bundle rebuild.
- [ ] Step 1: failing unit/e2e tests; Step 2: implement; Step 3: `npm --prefix ui run lint|check|test|e2e`, `just ui-build`, `just ui-dist-check`, run `perf-scroll` once (its own config) and record numbers. Commits: `fix(ui): browser-safe intercept chord, derived shortcut hints, view consistency` and `build(ui): rebuild embedded bundle`.

### Task 4: full gate, review, merge
- [ ] `just lint`, `just doc`, `cargo deny check`, `just features`, `cargo test --workspace --locked --all-features --no-fail-fast` (background log), UI lint/check/test/e2e, `just first-capture-ui`; whole-branch review (opus); PR with staged body; CI; merge; update `docs/security-gates.md` status if needed; backlog memory.
