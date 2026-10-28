# Performance Harness Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the four §13 performance gates Plan 6 didn't: proxy added-latency, proxy throughput (+ SNI tunnel), ingest rate on the real writer path, and the UI frame/filter trace.

**Architecture:** An in-process Rust load harness (upstream echo + K keep-alive drivers + latency/throughput collection) drives the two proxy gates; an ingest bench feeds the real store writer; a Playwright trace covers the UI; all wired into the existing maintainer-run `just perf` plus a reduced-scale CI `perf-smoke`, with baselines in `bench/README.md`. Tight numbers are maintainer-reference; CI asserts only generous build/regression bounds.

**Tech Stack:** Rust (crosswire-proxy, crosswire-store, crosswire-server), the existing proxy/store test harnesses, the Plan 6 synthetic generator, Playwright, GitHub Actions.

**Spec:** docs/superpowers/specs/2026-10-28-crosswire-performance-harness-design.md

## Global Constraints

- `unsafe_code = "forbid"`; MSRV 1.88; rustls ring-only; `cargo build` Node-free.
- Conventional Commits, ≤72-char subjects, **no attribution trailers** (verify `git log -1 | grep -c -i claude` == 0 after each commit).
- `cargo deny` clean; NO new product dependency; a dev-dependency only if justified + lockfile + deny-clean (prefer std + existing test deps). `oha` is NOT a required/CI tool — document it only.
- Additive only: no wire/schema change; NO limit-value change; no product code change unless a harness reveals a real regression (then fix in the owning crate + note).
- Hardware-sensitive numbers are **maintainer-run** (`just perf`, both reference machines in `bench/README.md`); CI `perf-smoke` runs reduced scale with GENEROUS bounds (≈10× reference) that pass on noisy runners yet fail a gross regression — must not flake.
- FULL GATE before a task is done: `just lint`, `just doc`, `cargo deny check`, `cargo test` (the relevant crate); CI/justfile additions must run.
- Reuse the Plan 6 perf pattern (`scripts/perf-gate.sh`, `bench/README.md`, `just perf`/`perf-smoke`) and the `synthetic` generator.

## Review Focus

- Added-latency must be apples-to-apples (same client/upstream/body/keep-alive; only the proxy hop differs) — else the delta is meaningless (Task 1).
- The load must actually SATURATE (enough concurrency/duration) so req/s is throughput, not a latency-bound trickle (Task 0 + Task 2).
- Ingest must count COMMITTED exchanges through the real queue→writer→commit path, not enqueued/bulk-loaded (Task 3).
- The §13 body-mix proportions (60/30/9/1) must be the ones measured, not a uniform size (Task 3).
- The UI trace must scroll a REAL 1M project (not an empty view); filter timing is input→rendered-result (Task 4).

---

### Task 0: in-process load harness

**Files:**
- Create: `crates/crosswire-proxy/tests/load/mod.rs` (test-support; included by the gate binaries via `#[path = "load/mod.rs"] mod load;`)
- Test: `crates/crosswire-proxy/tests/load_selftest.rs`

**Interfaces:**
- Produces:
  - `pub struct Stats { pub p50_us: u64, pub p99_us: u64, pub reqs: u64, pub elapsed: Duration }` with `pub fn reqs_per_sec(&self) -> f64`.
  - `pub async fn spawn_echo_upstream(body_len: usize) -> SocketAddr` — a keep-alive HTTP/1.1 upstream returning a `body_len`-byte body.
  - `pub async fn drive(target: Target, conns: usize, duration: Duration, body_len: usize) -> Stats` where `enum Target { Direct(SocketAddr), ViaProxy { proxy: SocketAddr, upstream_authority: String } }` — drives `conns` keep-alive connections issuing requests for `duration`, collecting per-request latency into a histogram → p50/p99 + req/s.
- Consumes: the proxy test harness (`ProxyHarness`) for spawning a real proxy where a later task needs one.

- [ ] **Step 1: Write the failing test** `harness_measures_direct_latency_and_throughput`: spawn the echo upstream; `drive(Direct(addr), 8, 1s, 16KiB)`; assert `stats.reqs > 0`, `p99_us >= p50_us > 0`, `reqs_per_sec() > 0`. Add `percentiles_are_ordered` on a synthetic latency set (p50 ≤ p99).
- [ ] **Step 2: Run** `cargo test -p crosswire-proxy --test load_selftest` → FAIL.
- [ ] **Step 3: Implement** `load/mod.rs`. Use `tokio` tasks for the K connections; collect latencies in a `Vec<u64>` per connection, merge + sort for percentiles (no new dep). The upstream is a minimal keep-alive responder.
- [ ] **Step 4: Run** → PASS.
- [ ] **Step 5: FULL GATE.**
- [ ] **Step 6: Commit** `test(proxy): in-process load harness for perf gates`.

---

### Task 1: added-latency gate (`< 2 ms`)

**Files:**
- Create: `crates/crosswire-proxy/tests/perf_added_latency.rs`
- Modify: `bench/README.md` (added-latency baseline slot)

**Interfaces:**
- Consumes: `load::{drive, spawn_echo_upstream, Target, Stats}` (Task 0); `ProxyHarness` for a real MITM+recording proxy.

- [ ] **Step 1: Write the test** `added_latency_under_2ms` (`#[tokio::test(flavor="multi_thread", worker_threads=4)]`): spawn the echo upstream (16 KiB); measure `p99_direct = drive(Direct, 64, DUR, 16KiB)`; start a real proxy (MITM on, recording on) and measure `p99_proxy = drive(ViaProxy, 64, DUR, 16KiB)`; assert `p99_proxy - p99_direct < 2000 µs`. Print both p99s + the delta as a `SUMMARY` line. On a host too noisy for the tight bound, the test still runs and prints; the ASSERT is gated behind an env flag (`CROSSWIRE_PERF_ASSERT=1`, set by `just perf`) so it reports under CI smoke but only asserts the reference number maintainer-side.
- [ ] **Step 2: Run** `cargo test -p crosswire-proxy --test perf_added_latency` → it runs; the assertion is active only with `CROSSWIRE_PERF_ASSERT=1`.
- [ ] **Step 3: Implement** the test using Task 0's harness; ensure the direct and via-proxy runs are identical except for the proxy hop.
- [ ] **Step 4: Run** with `CROSSWIRE_PERF_ASSERT=1` locally (report the delta; if a real regression makes the proxy hop >2 ms, STOP and report — do not weaken).
- [ ] **Step 5: FULL GATE**, record the M-series delta in `bench/README.md`.
- [ ] **Step 6: Commit** `test(proxy): added-latency perf gate under 2ms`.

---

### Task 2: throughput gate (`≥ 5k req/s`) + SNI-tunnel row

**Files:**
- Create: `crates/crosswire-proxy/tests/perf_throughput.rs`
- Modify: `bench/README.md`

**Interfaces:**
- Consumes: `load::{drive, Stats}` (Task 0); `ProxyHarness` (MITM+recording, and a CONNECT SNI-passthrough path).

- [ ] **Step 1: Write the tests** (`multi_thread`, 4 workers, env-gated assert as in Task 1): `throughput_mitm_at_least_5k` — saturate the proxy (MITM + recording, enough conns/duration to saturate); assert `reqs_per_sec() >= 5000` (under `CROSSWIRE_PERF_ASSERT=1`); print a `SUMMARY` row. `throughput_sni_tunnel` — drive an HTTPS CONNECT SNI-passthrough tunnel (no MITM); REPORT its req/s as its own row (no hard assert — it's a reference row). 
- [ ] **Step 2: Run** `cargo test -p crosswire-proxy --test perf_throughput` → runs; MITM assert active only with the env flag.
- [ ] **Step 3: Implement** using the harness; pick conns/duration that saturate the proxy on the reference machine.
- [ ] **Step 4: Run** with `CROSSWIRE_PERF_ASSERT=1`; a real sub-5k regression → STOP + report.
- [ ] **Step 5: FULL GATE**, record both rows in `bench/README.md`.
- [ ] **Step 6: Commit** `test(proxy): throughput perf gate and SNI-tunnel row`.

---

### Task 3: ingest gate (`≥ 2k exchanges/s`, real writer path)

**Files:**
- Create: `crates/crosswire-store/tests/perf_ingest.rs` (or a bench)
- Modify: `bench/README.md`

**Interfaces:**
- Consumes: the real store writer (the bounded queue → OS writer thread → `committed`; find the ingest/submit API used by the proxy sink, NOT the synthetic bulk-insert); the Plan 6 §13 body-mix definition (reuse the proportions 60 % < 4 KiB, 30 % 4–64 KiB, 9 % 64 KiB–1 MiB, 1 % > 1 MiB).

- [ ] **Step 1: Write the test** `ingest_at_least_2k_per_sec` (env-gated assert): open a real store; submit exchanges at the §13 body mix through the real ingest path for a fixed window (e.g. 3 s or N exchanges); count **committed** exchanges (wait for the `committed` signal / poll the store), compute committed/s; assert `>= 2000` under `CROSSWIRE_PERF_ASSERT=1`. Print a `SUMMARY`. Assert the body-mix proportions of what was submitted are within tolerance of 60/30/9/1.
- [ ] **Step 2: Run** `cargo test -p crosswire-store --test perf_ingest` → runs.
- [ ] **Step 3: Implement** on the real queue→writer path (reuse how `crosswire-server`/the proxy sink submits exchanges). A real sub-2k regression → STOP + report.
- [ ] **Step 4: Run** with `CROSSWIRE_PERF_ASSERT=1`.
- [ ] **Step 5: FULL GATE**, record the M-series ingest rate in `bench/README.md`.
- [ ] **Step 6: Commit** `test(store): ingest perf gate on the real writer path`.

---

### Task 4: UI frame/filter trace (`no frame > 32 ms`, `filter < 300 ms`)

**Files:**
- Create: `ui/e2e/perf-scroll.spec.ts`
- Modify: a justfile recipe and/or a small script to build + serve a 1M synthetic project for the trace

**Interfaces:**
- Consumes: the Plan 6 `synthetic` generator (`cargo run`/a helper that builds a 1M-row project) + a real `crosswire serve` on that project; Playwright tracing (`page.context().tracing` or performance entries).

- [ ] **Step 1: Write the e2e** `perf-scroll.spec.ts`: against a served 1M-row project, open the history view; capture a trace over a 10 s programmatic scroll; assert NO animation frame exceeds 32 ms (from the trace / `requestAnimationFrame` deltas); apply a filter and assert the result renders in < 300 ms (input → rendered rows). Gate the tight assertions behind an env flag (`CROSSWIRE_PERF_ASSERT`) so CI smoke runs it (generous bound) and the maintainer asserts the reference numbers.
- [ ] **Step 2: Run** the spec against a built 1M project → it runs.
- [ ] **Step 3: Implement** the project-build/serve helper (reuse the synthetic generator; the 1M build is slow — allow it, or a reduced row count for CI smoke via an env var) and the spec. A real frame>32 ms or filter>300 ms regression → STOP + report.
- [ ] **Step 4: Run** `npm --prefix ui run e2e` for the new spec (reduced rows locally if 1M is too slow; note it).
- [ ] **Step 5: FULL GATE** (UI lint/test/e2e), record the frame/filter baseline in `bench/README.md`.
- [ ] **Step 6: Commit** `test(ui): frame-time and filter perf trace at 1M rows`.

---

### Task 5: wire `just perf` + CI perf-smoke + baselines + §13 ledger

**Files:**
- Modify: `justfile` (extend `perf` and `perf-smoke`), `.github/workflows/ci.yml` (the proxy/ingest smokes on the `check` job, the UI trace smoke on the UI-e2e job), `bench/README.md` (the four baselines + the §13 perf ledger + the oha cross-check note), `scripts/perf-gate.sh` only if a shared assertion helper is reused

**Interfaces:**
- Consumes: the four gate tests/benches (Tasks 1–4), each of which prints a `SUMMARY` and asserts under `CROSSWIRE_PERF_ASSERT=1`.

- [ ] **Step 1: Extend `just perf`** to run the Plan 6 storage gate PLUS the four new gates with `CROSSWIRE_PERF_ASSERT=1` (full scale), printing each SUMMARY with a compare-to-`bench/README.md` pointer.
- [ ] **Step 2: Extend `just perf-smoke`** to run each new harness at reduced scale WITHOUT `CROSSWIRE_PERF_ASSERT` (or with a generous ≈10× bound) — proving it builds/runs, not the reference number. Prove it: a deliberately broken harness (e.g. a panic) fails perf-smoke; the current tree passes.
- [ ] **Step 3: Wire CI** — the proxy/ingest smokes into the ubuntu `check` job, the UI trace smoke into the ubuntu UI-e2e job (reduced rows); pin no new action; validate the YAML.
- [ ] **Step 4: Run** `just perf-smoke` locally → green; `python3 -c "import yaml; yaml.safe_load(open('.github/workflows/ci.yml'))"`.
- [ ] **Step 5: Write `bench/README.md`** — the four M-series baselines, a labeled 4-vCPU-Linux-VM slot, the §13 perf-gate ledger (which harness covers which gate; tight numbers maintainer-reference, CI smoke generous), and the oha optional-cross-check note.
- [ ] **Step 6: FULL GATE**, then **Commit** `ci: wire the performance gates into perf and perf-smoke`.

---

## Notes for the executor

- Order 0 → 1 → 2 → 3 → 4 → 5. Task 0 produces the harness Tasks 1–2 consume; Tasks 3 (store) and 4 (UI) are independent; Task 5 wires them all.
- Suggested models: Task 0 (harness, concurrency) → opus; Tasks 1–3 (apply harness, bench) → sonnet; Task 4 (Playwright trace) → sonnet; Task 5 (wiring) → sonnet. Escalate to opus if a harness surfaces a real perf regression needing a product fix.
- The **env-gated assert** pattern (`CROSSWIRE_PERF_ASSERT=1`) is the key to running under CI smoke (report-only) while asserting the reference numbers maintainer-side — carry it consistently across Tasks 1–4.
- Do NOT change any limit or product behavior for performance. If a harness shows a real regression, that's the gate working — fix in the owning crate + note + keep the harness.
- The 1M synthetic build (Task 4) is slow; use a reduced row count for the CI smoke via an env var, full 1M for the maintainer run.
