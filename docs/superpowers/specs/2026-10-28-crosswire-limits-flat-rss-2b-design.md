# Crosswire Limits flat-RSS matrix 2b — design spec

> **Status:** third "Harden & prove (alpha.2 DoD)" sub-plan (2b). Completes §13
> security gate #18 ("one hostile fixture per Limits row with flat RSS"). Authority:
> the product spec `docs/superpowers/specs/2026-10-22-crosswire-design.md` (§11.6
> Limits table, §13). Where this document and the product spec disagree, the product
> spec wins.

## 1. Goal

Prove that every §11.6 Limits row where an attacker's input could drive memory holds
**flat RSS** under attack — the process's anonymous RSS does not grow materially while
a hostile input is driven to its cap and refused/truncated. Today only one row
(`until-close response`, `limits_rss.rs::rss_flat_under_large_until_close_response`)
has such an assertion; the rest of `limits.rs` asserts refusal behavior but not memory.

## 2. Scope

**In scope — flat-RSS assertions for the memory-relevant rows** (attacker input can
drive allocation):
- Parse limits (`ParseLimits`, §5.1 / §11.6): head size (64 KiB), header count (256),
  line length (16 KiB), chunk count (`max_chunks` 1M), chunk-size line
  (`max_chunk_size_line` 4 KiB), trailer size (`max_trailer_size` 16 KiB).
- `capture_cap` (50 MiB) — body truncation sets `truncated_at`.
- In-flight body budget (256 MiB) — new bodies get `capture_dropped`.
- Decompression size / ratio (64 MiB / 100:1) — derived view marked truncated.
- `intercept_hold` (4 MiB) — held-request bytes cap.
- Store queue (64 MiB / 10k) — spill to journal, then drop with `gap`.

**Documented, not asserted (bounded by a counter, not a buffer):** pending-CONNECT
cap, per-client-IP cap, max connections, SNI length, DNS timeout/concurrency,
TLS-handshake timeout, handshake-rejection auto-passthrough, leaf-cert LRU/mint,
redirect-follow count, WS per-subscriber buffer, audit.jsonl cap, journal disk cap.
Each gets a one-line rationale in the gate ledger (its limit refuses/counts rather
than buffering attacker bytes, so RSS is flat by construction).

**Out of scope:** changing any limit value or product behavior (unless a fixture
surfaces a real unbounded-growth defect — then fix it in the owning crate); the
preview zero-egress layer (separate sub-plan); non-memory gates.

## 3. Global constraints (verbatim from the product spec)

- `unsafe_code = "forbid"`; MSRV 1.88; rustls ring-only; `cargo build` Node-free.
- Conventional Commits, ≤72-char subjects, no attribution trailers.
- `cargo deny` clean; no new external dependency (std `/proc` + existing harness).
- Additive only: no wire/schema/query-semantics change; no limit-value change.
- FULL GATE before a task is done: `just lint`, `just doc`, `cargo deny check`,
  `cargo test` (the relevant crate).

## 4. The shared flat-RSS harness

A reusable test helper (the foundation every row uses):

- **RssAnon probe:** reuse `crosswire-store`'s `parse_rss_anon` / `anon_rss_bytes`
  (Linux `/proc/self/status` `RssAnon`; excludes mmap'd files — the right metric).
  On macOS fall back to the `ps rss` the existing `limits_rss.rs` uses, or skip with
  a notice (the CI gate runs on ubuntu). Expose the probe to both proxy and store
  test crates without a new dependency (a shared `#[path]`-included test module, or a
  tiny probe duplicated as the existing `limits_rss.rs` already does — pick one and
  keep it DRY within each crate).
- **Isolation contract:** RSS is process-global, so flat-RSS tests must not run
  concurrently with other allocating tests. Provide a process-global guard — a
  `static Mutex` each flat-RSS test acquires for its whole measured section — so a
  waiting test allocates nothing while another measures. One such guard per test
  binary; grouping multiple rows into one binary is allowed **only** behind the guard.
- **The assertion helper:** `fn with_flat_rss(cap_mib: u64, drive: impl FnOnce ...)`
  that (a) acquires the guard, (b) samples baseline RssAnon, (c) runs a background
  sampler recording peak, (d) runs `drive` (which issues the hostile input and awaits
  the refusal/truncation), (e) asserts `peak - baseline < cap_mib`. The cap is chosen
  per row well below what an unbounded implementation would reach (e.g. parse-limit
  rows: a few MiB cap while the attacker sends hundreds of MiB; capture_cap: growth
  bounded near the 50 MiB cap not the attacker's multi-hundred-MiB body).

## 5. Per-row fixtures (grouped by owning subsystem)

Each fixture **drives a hostile input that a naive implementation would buffer
unboundedly**, confirms the existing refusal/truncation behavior still holds, and
wraps the driving section in `with_flat_rss`. Reuse the existing driving harnesses
(`limits.rs::through_tunnel`/`head_refused`/`body_refused`, `limits_rss.rs::plain_rig`).

- **Group A — proxy parse limits:** head size, header count, line length, chunk
  count, chunk-size line, trailer size. The attacker streams far more than the limit;
  RSS stays near `limit + tail`, not the sent volume.
- **Group B — proxy body limits:** `capture_cap` (a body far over 50 MiB → RSS bounded
  near the cap, `truncated_at` set); decompression ratio/size (a zstd/gzip bomb whose
  decoded view is capped at 64 MiB / 100:1 → RSS bounded, banner); in-flight body
  budget (enough concurrent large bodies to exceed 256 MiB → new ones
  `capture_dropped`, RSS bounded at the budget).
- **Group C — store / intercept:** store queue (flood past 64 MiB/10k → spill to
  journal then `gap`, RSS bounded at the queue cap, not the flood); `intercept_hold`
  (a held request far over 4 MiB → held bytes bounded at the cap).

If a row's fixture shows RSS tracking the attacker's volume instead of the cap, that
is a real unbounded-growth defect — fix it in the owning crate and note it.

## 6. Review focus (inputs the fixtures must genuinely exercise)

- The hostile input must be **materially larger** than the cap (e.g. ≥ 10× or
  hundreds of MiB) so a broken bound would blow the RSS cap and fail the test — a
  fixture that sends just over the limit proves nothing about flatness.
- The sampler must run **during** the drive, not only before/after, so a transient
  spike is caught.
- `capture_cap` / decompression / budget must assert the observable (`truncated_at` /
  banner / `capture_dropped`) AND flat RSS — behavior and memory together.
- The guard must actually serialize: two flat-RSS tests in one binary must not
  interleave their measured sections.
- macOS skip must be clean (no false pass); the real assertion runs on CI-Linux.

## 7. Acceptance

- Each memory-relevant row has a flat-RSS assertion that would FAIL if its bound
  were removed (non-vacuous: the hostile input is far over the cap).
- The counter/timeout rows are enumerated in `docs/security-gates.md` #18 with a
  one-line bounded-by-construction rationale each.
- `docs/security-gates.md` #18 → proven, citing the row coverage.
- No limit value or product behavior changed (unless a real unbounded-growth defect
  was found + fixed, noted in the commit).
- `unsafe_code = "forbid"` held; no new dependency; `cargo deny` clean; full gate green.
