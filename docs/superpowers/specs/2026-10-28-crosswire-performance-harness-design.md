# Crosswire performance harness — design spec

> **Status:** the last big "alpha.2 DoD" block. Closes the §13 performance gates not
> covered by Plan 6 (which did storage/query/RSS): proxy added-latency, proxy
> throughput, ingest rate, and the UI frame/filter trace. Authority: the product spec
> `docs/superpowers/specs/2026-10-22-crosswire-design.md` (§13 Performance, §9.3 write
> path). Where this document and the product spec disagree, the product spec wins.

## 1. Goal

Prove the §13 performance numbers with real, repeatable harnesses:

- **Added latency:** `p99(via proxy) − p99(direct)` on loopback **< 2 ms**, with MITM
  TLS on, recording on, 64 keep-alive connections, a 16 KiB body.
- **Throughput:** **≥ 5,000 req/s** with MITM and recording on, plus a **separate
  SNI-tunnel (passthrough) row**.
- **Ingest:** **≥ 2,000 exchanges/s** through the **real writer path** at the §13 body
  mix (60 % < 4 KiB, 30 % 4–64 KiB, 9 % 64 KiB–1 MiB, 1 % > 1 MiB).
- **UI:** no frame **> 32 ms** in a Playwright trace over a 10 s scroll at 1M rows;
  filter applied **< 300 ms** at 1M rows.

These are **hardware-sensitive**, so the numeric gate is **maintainer-run on both
reference machines** (an Apple M-series laptop and a 4-vCPU Linux VM, per §13), via an
extended `just perf`. CI runs a **reduced-scale smoke** that proves the harness builds
and runs and catches gross regressions, but does **not** assert the reference numbers
(CI hardware ≠ reference machines) — mirroring Plan 6's `just perf` / `perf-smoke`.

## 2. Scope

**In scope:** the four gates above; an in-process Rust load harness; the ingest bench
on the real writer path; the UI Playwright frame/filter trace; wiring into
`just perf` + CI `perf-smoke`; baselines in `bench/README.md`; the §13 perf ledger.

**Out of scope:** changing any product code for performance (unless a harness reveals a
real regression — then fix + note); HTTP/2 throughput (v0.2a+); the storage/query/RSS
gates (done in Plan 6); `oha` as a required/CI tool.

## 3. Global constraints (verbatim from the product spec)

- `unsafe_code = "forbid"`; MSRV 1.88; rustls ring-only; `cargo build` Node-free.
- Conventional Commits, ≤72-char subjects, no attribution trailers.
- `cargo deny` clean; no new external **product** dependency; a dev-dependency is
  allowed only if justified + lockfile + deny-clean (prefer std + existing test deps).
- Additive only: no wire/schema change; no limit-value change.
- FULL GATE before a task is done: `just lint`, `just doc`, `cargo deny check`,
  `cargo test` (the relevant crate); the CI smoke additions must themselves run.
- Reuse the existing proxy/store integration harnesses (`crosswire-proxy/tests`,
  `crosswire-server/tests/gates.rs`, the synthetic generator from Plan 6) and the
  Plan 6 perf pattern (`scripts/perf-gate.sh`, `bench/README.md`, `just perf`).

## 4. The in-process load harness

A reusable Rust load generator (a `crosswire-proxy` bench or test-support module):

- Spawns an **in-process upstream echo server** (returns a fixed 16 KiB body under
  keep-alive) on loopback.
- Drives **K keep-alive connections** (default 64) issuing requests back-to-back for a
  fixed duration or request count, each recording per-request wall-clock latency.
- Produces **p50 / p99 latency** and **req/s** from the collected samples.
- Runs the load **two ways against the same upstream**: (a) **direct** (client →
  upstream) and (b) **via the proxy** (client → `crosswire serve` proxy → upstream),
  with MITM TLS on and recording on for (b). The added-latency gate is
  `p99(b) − p99(a)`.
- Deterministic and `oha`-free. (`oha` is documented in `bench/README.md` as an
  optional maintainer macro cross-check for raw throughput, not run by the gate.)

## 5. The four gates

### 5.1 Added latency (`< 2 ms`)
Drive the harness direct vs via-proxy (MITM on, recording on, 64 keep-alive, 16 KiB
body) and assert `p99_proxy − p99_direct < 2 ms`. Report both p99s and the delta.

### 5.2 Throughput (`≥ 5k req/s`) + SNI-tunnel row
Saturate the proxy (MITM + recording) and measure req/s; assert `≥ 5000`. A **second
measurement** drives an HTTPS CONNECT **SNI-passthrough tunnel** (no MITM) and reports
its req/s as its own row. Both to `bench/README.md`.

### 5.3 Ingest (`≥ 2k exchanges/s`, real writer path)
Feed exchanges through the **real store writer** (the bounded queue → OS writer thread
→ `committed`, not the synthetic bulk-insert) at the §13 body mix, and measure
sustained `committed` exchanges/s over a fixed window; assert `≥ 2000`. Reuse the Plan 6
body-mix definition. This exercises the §9.3 write path (batch commit every 250 ms /
1,000 exchanges), not a bulk load.

### 5.4 UI frame/filter trace (`no frame > 32 ms`, `filter < 300 ms`)
A Playwright test over a served **1M-row synthetic project** (built via the Plan 6
`synthetic` generator + a real `crosswire serve`): capture a trace over a **10 s scroll**
of the history view and assert **no frame > 32 ms**; apply a filter and assert it
renders in **< 300 ms**. Runs in the ubuntu UI-e2e job but, like the other perf gates,
its tight numbers are maintainer-reference; CI asserts a generous bound + that it runs.

## 6. Gate wiring

- **`just perf`** (extended, maintainer-run): the Plan 6 storage gate PLUS the proxy
  latency/throughput, ingest, and (where runnable headless) the UI trace — each with its
  threshold, printing a SUMMARY to compare against `bench/README.md`.
- **CI `perf-smoke`** (extended): runs each harness at **reduced scale/duration** with
  **generous** bounds (proving it builds, runs, and isn't grossly broken) — NOT the
  reference numbers. The proxy/ingest smokes run on the ubuntu `check` job; the UI
  trace smoke on the ubuntu UI-e2e job.
- **`bench/README.md`**: add the proxy added-latency, throughput (+ SNI row), ingest,
  and UI frame/filter baselines, recorded on the M-series reference; leave a clearly
  labeled slot for the 4-vCPU Linux VM reference (filled by a maintainer run / CI).

## 7. Review focus (inputs the harnesses must genuinely exercise)

- The added-latency measurement must be **apples-to-apples**: same client, same
  upstream, same body, same keep-alive count — the only difference is the proxy hop
  (else the delta is meaningless).
- The load must actually **saturate** (enough concurrency/duration) so req/s is a real
  throughput, not a latency-bound trickle.
- Ingest must go through the **real queue→writer→commit** path and count **committed**
  exchanges (not enqueued), so it measures durable write rate, not buffering.
- The §13 body mix proportions must be the ones measured, not a uniform size.
- The UI trace must scroll a **real 1M project** (not an empty view) so the frame times
  mean something; the filter timing is measured from input to rendered result.
- CI smokes must assert **generous** bounds that pass on noisy shared runners yet still
  fail on a gross regression (e.g. 10× the reference), and must not flake.

## 8. Acceptance

- Each gate has a harness that produces its metric and asserts its threshold under
  `just perf`; the CI `perf-smoke` runs each at reduced scale with generous bounds.
- `bench/README.md` records the M-series baselines for all four, with a labeled Linux-VM
  slot.
- No product code changed (unless a real regression was found + fixed + noted).
- `unsafe_code = "forbid"` held; no new product dependency; `cargo deny` clean; full
  gate green.
- The §13 perf ledger (in `bench/README.md` or `docs/`) records which gate each harness
  covers and that the tight numbers are maintainer-reference, not CI-asserted.
