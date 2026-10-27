# Limits flat-RSS matrix 2b Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Complete §13 gate #18 — flat-RSS assertions for the ~10 memory-relevant §11.6 Limits rows, plus a documented bounded-by-construction rationale for the counter/timeout rows.

**Architecture:** A shared flat-RSS test harness (RssAnon probe + a process-global serialization guard + a `with_flat_rss(cap, drive)` assertion) that every row reuses; then three grouped test binaries — proxy parse-limits, proxy body-limits, store/intercept — each driving a hostile input far over its cap and asserting RSS stays near the cap. Tests only, unless a fixture surfaces a real unbounded-growth defect.

**Tech Stack:** Rust (crosswire-proxy, crosswire-store tests), std `/proc` RssAnon, existing proxy/store test harnesses.

**Spec:** docs/superpowers/specs/2026-10-28-crosswire-limits-flat-rss-2b-design.md

## Global Constraints

- `unsafe_code = "forbid"`; MSRV 1.88; rustls ring-only; `cargo build` Node-free.
- Conventional Commits, ≤72-char subjects, **no attribution trailers** (verify `git log -1 | grep -c -i claude` == 0 after each commit).
- `cargo deny` clean; **no new external dependency** (std `/proc` + existing harness).
- Additive only: no wire/schema change; **no limit-value change**; no product behavior change unless a fixture finds a real unbounded-growth defect (then fix in the owning crate + note it).
- FULL GATE before a task is done: `just lint`, `just doc`, `cargo deny check`, `cargo test` (the relevant crate).
- RSS is process-global: every flat-RSS test runs behind the shared guard; grouping rows in one binary is allowed ONLY behind the guard.
- Each task updates its own rows in `docs/security-gates.md` (distinct text — no conflict under serialized execution); Task 3 writes the #18 "proven" summary + the counter-row rationale table.

## Review Focus

- Hostile input must be **materially larger** than the cap (≥10× or hundreds of MiB) so a removed bound blows the RSS cap and fails — a fixture sending just over the limit proves nothing (every task's fixtures).
- The RSS sampler must run **during** the drive, not only before/after, to catch a transient spike (Task 0 harness + every row).
- `capture_cap` / decompression / in-flight budget must assert the observable (`truncated_at` / banner / `capture_dropped`) AND flat RSS together (Task 2).
- The guard must actually serialize — two flat-RSS tests in one binary must not interleave their measured sections (Task 0 + any multi-test binary).
- macOS skip must be clean (no false pass); the real assertion runs on CI-Linux (Task 0 + every row).

---

### Task 0: shared flat-RSS harness

**Files:**
- Create: `crates/crosswire-proxy/tests/flat_rss/mod.rs` (a test-support module; include into each flat-RSS binary via `#[path = "flat_rss/mod.rs"] mod flat_rss;`, mirroring how the client tests share `common/mod.rs`)
- Reference: `crosswire-store/src/rss.rs` (`parse_rss_anon`, `anon_rss_bytes`) and the existing `crosswire-proxy/tests/limits_rss.rs::rss_kib` for the macOS fallback pattern.

**Interfaces:**
- Produces:
  - `pub fn rss_anon_kib() -> Option<u64>` — Linux `/proc/self/status` `RssAnon` (NOT `VmRSS`); macOS `ps rss` fallback (as `limits_rss.rs` does); `None` elsewhere.
  - `pub async fn with_flat_rss<F, Fut>(cap_mib: u64, drive: F) where F: FnOnce() -> Fut, Fut: Future<Output=()>` — acquires a process-global `static Mutex`, samples baseline `rss_anon_kib`, spawns a background sampler recording peak via `AtomicU64::fetch_max`, runs `drive().await`, stops the sampler, asserts `peak - baseline < cap_mib * 1024` with a message printing baseline/peak/growth. On a host where `rss_anon_kib()` is `None`, it prints a SKIP notice and runs `drive` without asserting (so macOS passes; CI-Linux asserts).

- [ ] **Step 1: Write the failing test** `flat_rss_harness_catches_growth` (in a small `tests/flat_rss_selftest.rs` that includes the module): call `with_flat_rss(4, || async { let _v: Vec<u8> = vec![0u8; 64*1024*1024]; /* hold across a sample */ tokio::time::sleep(50ms).await; drop(_v); })` and assert (on Linux) that it PANICS/fails the inner assertion (64 MiB > 4 MiB cap); and `with_flat_rss(64, || async { no allocation })` passes. Use `std::panic::catch_unwind` / a result channel to observe the inner assertion firing. On macOS, assert the SKIP path returns without asserting.
- [ ] **Step 2: Run** `cargo test -p crosswire-proxy --test flat_rss_selftest` → FAIL (module not present).
- [ ] **Step 3: Implement** `flat_rss/mod.rs`. Sampler: a `tokio::spawn` loop doing `spawn_blocking(rss_anon_kib)` every ~10ms into an `AtomicU64` peak, cancelled after `drive`. The guard is a `static GUARD: Mutex<()>` (or `tokio::sync::Mutex`) held for the whole measured section.
- [ ] **Step 4: Run** the selftest → PASS (growth caught on Linux; skip on macOS).
- [ ] **Step 5: FULL GATE.**
- [ ] **Step 6: Commit** `test(proxy): shared flat-RSS harness with a serialization guard`.

---

### Task 1: Group A — proxy parse-limits flat RSS

**Files:**
- Create: `crates/crosswire-proxy/tests/limits_rss_parse.rs` (its own binary; `#[path="flat_rss/mod.rs"] mod flat_rss;` + reuse `limits.rs`'s driving helpers by copying the minimal rig or importing a shared `common`)
- Modify: `docs/security-gates.md` (#18 — note parse-limit rows covered)

**Interfaces:**
- Consumes: `flat_rss::{with_flat_rss, rss_anon_kib}` (Task 0); the proxy test harness (`ProxyHarness`, `through_tunnel`, `head_refused`, `body_refused` patterns from `limits.rs` / `limits_rss.rs::plain_rig`).

- [ ] **Step 1: Write the failing tests** (one per row, each behind `with_flat_rss`): `head_size_flat_rss`, `header_count_flat_rss`, `line_length_flat_rss`, `max_chunks_flat_rss`, `chunk_size_line_flat_rss`, `trailer_size_flat_rss`. Each drives an input FAR over the limit (e.g. a multi-hundred-MiB stream of headers / a chunked body with millions of tiny chunks / a trailer section far over 16 KiB), asserts the existing refusal marker (`limit_exceeded` / the `ParseError`), AND asserts flat RSS with a cap a few MiB above `limit + tail`.
- [ ] **Step 2: Run** `cargo test -p crosswire-proxy --test limits_rss_parse` → FAIL.
- [ ] **Step 3: Implement** the six tests. If a row's RSS tracks the sent volume instead of the limit, that is a real defect — fix in `crosswire-h1`/`crosswire-proxy` and note it.
- [ ] **Step 4: Run** → PASS (Linux asserts; macOS skips the RSS leg).
- [ ] **Step 5: FULL GATE.**
- [ ] **Step 6: Commit** `test(proxy): flat-RSS for the parse-limit rows`.

---

### Task 2: Group B — proxy body-limits flat RSS

**Files:**
- Create: `crates/crosswire-proxy/tests/limits_rss_body.rs`
- Modify: `docs/security-gates.md` (#18 — note capture_cap / decompression / in-flight budget covered)

**Interfaces:**
- Consumes: `flat_rss::{with_flat_rss}` (Task 0); the proxy relay harness that records a response/exchange and exposes `truncated_at` / `capture_dropped` / the decompression banner; the existing `limits_rss.rs::plain_rig` until-close rig.

- [ ] **Step 1: Write the failing tests:** `capture_cap_flat_rss` (relay a body far over 50 MiB → assert `truncated_at` set AND RSS bounded near 50 MiB, not the sent size), `decompression_ratio_flat_rss` (a zstd/gzip bomb whose decoded view is capped at 64 MiB / 100:1 → assert the derived view is marked truncated/banner AND RSS bounded), `inflight_budget_flat_rss` (drive enough concurrent large bodies to exceed the 256 MiB in-flight budget → assert new bodies get `capture_dropped` AND RSS bounded at the budget). Each behind `with_flat_rss` with a cap set near the row's own cap, well below the attacker's volume.
- [ ] **Step 2: Run** `cargo test -p crosswire-proxy --test limits_rss_body` → FAIL.
- [ ] **Step 3: Implement.** Reuse the decompression and capture machinery; the bomb fixtures must decode to far more than the cap. A real unbounded-growth finding → fix + note.
- [ ] **Step 4: Run** → PASS.
- [ ] **Step 5: FULL GATE.**
- [ ] **Step 6: Commit** `test(proxy): flat-RSS for capture, decompression, in-flight budget`.

---

### Task 3: Group C — store/intercept flat RSS + the #18 ledger

**Files:**
- Create: `crates/crosswire-store/tests/limits_rss_store.rs` (store queue) and extend an intercept test for `intercept_hold` (in `crosswire-proxy/tests/` where intercept hold lives — `intercept.rs` or a new `limits_rss_intercept.rs`). Each gets its own flat-RSS include (store crate needs its own copy of the harness or reuse `rss::anon_rss_bytes` directly with a local guard — keep DRY within the crate).
- Modify: `docs/security-gates.md` (#18 → proven: the full row table with per-row status; counter/timeout rows each get a one-line bounded-by-construction rationale)

**Interfaces:**
- Consumes: `crosswire-store::rss::anon_rss_bytes` (store crate) + a local serialization guard (mirror Task 0's guard in the store test); the store ingest/queue path; the intercept hold path (`crosswire-proxy` `Intercept`).

- [ ] **Step 1: Write the failing tests:** `store_queue_flat_rss` (flood ingest far past the 64 MiB / 10k queue → assert spill-to-journal then `gap`, AND RSS bounded at the queue cap, not the flood volume), `intercept_hold_flat_rss` (hold a request far over 4 MiB → assert held bytes bounded at the 4 MiB cap, not the sent size). Each behind the serialization guard with the RSS assertion.
- [ ] **Step 2: Run** the two crates' new tests → FAIL.
- [ ] **Step 3: Implement.** A real unbounded-growth finding → fix in the owning crate + note.
- [ ] **Step 4: Run** `cargo test -p crosswire-store` and `cargo test -p crosswire-proxy` (the new binaries) → PASS.
- [ ] **Step 5: Write the #18 ledger:** `docs/security-gates.md` #18 → proven, with a table: each memory-relevant row → its flat-RSS test; each counter/timeout row (pending-CONNECT, per-IP, max-conns, SNI, DNS, TLS-handshake, handshake-rejection, leaf-LRU/mint, redirect, WS buffer, audit cap, journal cap) → a one-line "bounded by a counter/refusal, not a buffer" rationale.
- [ ] **Step 6: FULL GATE**, then **Commit** `test(store): flat-RSS for the queue and intercept hold; prove #18`.

---

## Notes for the executor

- Order 0 → 1 → 2 → 3; Task 0 produces the harness the others consume. Tasks 1–3 are otherwise independent (distinct binaries; distinct security-gates.md text).
- Suggested models: Task 0 (harness design + the serialize guard) → opus; Tasks 1–3 (apply the harness to rows, mechanical once the harness exists) → sonnet, EXCEPT escalate to opus if a fixture surfaces a real unbounded-growth defect needing a product fix.
- Every flat-RSS test must make the hostile input FAR over the cap (the Review-Focus point) — a borderline input is a defect in the test, not a pass.
- Do NOT change any limit value. If RSS tracks attacker volume, fix the bound in the owning crate and keep the fixture that found it.
