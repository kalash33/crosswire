# History benchmark baseline

The named history queries (spec §13) over a deterministic synthetic project,
measured by `crates/crosswire-store/benches/history.rs`:

```sh
# bench-sized (100 000 rows, criterion sampling plus the SUMMARY block)
cargo bench -p crosswire-store --features synthetic --bench history
# the 1 000 000-row gate (few runs per query; the load alone is ~3 minutes)
CROSSWIRE_BENCH_QUICK=1 CROSSWIRE_BENCH_N=1000000 \
  cargo bench -p crosswire-store --features synthetic --bench history
```

The bench prints `SUMMARY ...` lines (p95 per query, warm and cold, scroll
time, anonymous RSS start and peak). Targets: structured queries p95 < 50 ms
warm and < 500 ms cold; peak anonymous RSS < 500 MB at 1 000 000 rows.

* Warm: repeated on the pooled, already-open reader connection.
* Cold: pooled connections dropped before each run, so a fresh connection with
  an empty SQLite page cache. The OS page cache is not dropped.
* RSS is anonymous memory only (`RssAnon` on Linux, `phys_footprint` on macOS),
  never `VmRSS`, so the mmap'd database file is excluded.

## Reference machine: MacBook Air (Mac14,2), Apple M2, 8 cores, 8 GiB, macOS

Measured 2026-10-27, seed 7, release profile. The 100 000-row table below predates the v5 index change (schema v4).

### N = 100 000 (200 warm and 20 cold runs per query)

| query | kind | warm p95 (ms) | cold p95 (ms) |
|---|---|---|---|
| host | structured | 4.0 | 3.0 |
| status>=500 and POST | structured | 17.1 | 22.2 |
| min_resp_len=1048577 | structured | 30.9 | 64.6 |
| tag=Y | structured | 12.0 | 7.0 |
| in-scope | structured | 6.7 | 3.7 |
| path_contains | path scan | 2.6 | 24.2 |
| path_regex | path scan | 23.4 | 34.1 |

Scroll (10 pages of 200): 11.2 ms. Peak anonymous RSS 102 MB (includes
criterion's own sampling buffers). Load 13.3 s.

### N = 1 000 000 (quick mode: 8 warm and 4 cold runs, so p95 is the max)

Schema v5 (covering `method_status` and `resp_len` indexes). All structured
queries meet the targets (warm < 50 ms, cold < 500 ms).

| query | kind | warm p95 (ms) | cold p95 (ms) |
|---|---|---|---|
| host | structured | 2.4 | 2.6 |
| status>=500 and POST | structured | 0.20 | 0.50 |
| min_resp_len=1048577 | structured | 0.95 | 1.6 |
| tag=Y | structured | 9.7 | 10.4 |
| in-scope | structured | 2.2 | 3.9 |
| path_contains | path scan | 1.7 | 4.2 |
| path_regex | path scan | 2.1 | 7.2 |

Scroll (10 pages of 200): 1.6 ms. Peak anonymous RSS 21 MB (bound 500 MB).
Load 260 s.

History of the fix: with the v3 index `(req_method, resp_status, ulid)` the
`status>=500 and POST` query took 12 188 ms warm at 1M (temp b-tree sort after
fetching ~12 300 rows from the 5 GB primary tree). Reordering to
`(req_method, ulid, resp_status)` removed the sort (73 ms) but SQLite still
did a primary-tree lookup for every scanned entry; covering the list columns
brought it to 0.2 ms. `resp_len` had the same shape (34 to 49 ms, then the
planner flipped to a primary-key scan at 1M, 42 ms); it is covered too and
pinned with `INDEXED BY` (0.95 ms).

The 1 000 000 numbers are a single local quick-mode run on the reference
machine; the maintainer's full-gate run replaces them.

# Proxy added-latency gate (spec §13)

The proxy's steady-state **added** latency on the realistic HTTPS hot path
(MITM TLS on, recording on), measured by
`crates/crosswire-proxy/tests/perf_added_latency.rs` with the in-process load
harness (`crates/crosswire-proxy/tests/load/mod.rs`, `drive_mitm`):

```sh
# Report-only (CI smoke): runs, prints the SUMMARY line, never asserts.
cargo test --release -p crosswire-proxy --test perf_added_latency -- --nocapture
# Maintainer gate: additionally asserts delta < 2000 us.
CROSSWIRE_PERF_ASSERT=1 \
  cargo test --release -p crosswire-proxy --test perf_added_latency -- --nocapture
```

The gate drives 64 keep-alive connections issuing back-to-back origin-form
`GET /` for a 16 KiB response body over client TLS, once straight to a TLS echo
upstream and once through a real MITM proxy that mints a leaf, relays and
records; the per-connection TLS handshake and `CONNECT` setup (plus a
32-request warmup) are excluded from the samples. The `SUMMARY` line prints
both the p50 and p99 deltas; the maintainer gate **asserts on the p50 delta**
(`p50_proxy − p50_direct < 2 ms`, the intrinsic per-request proxy overhead).
Always built **`--release`** — debug rustls/ring crypto is ~20x slower and
meaningless here.

## Reference machine: MacBook Air (Mac14,2), Apple M2, 8 cores, 8 GiB, macOS

Measured 2026-10-28, release profile, 2 s per path.

| metric | direct | via MITM proxy | added (proxy − direct) |
|---|---|---|---|
| **p50 (asserted)** | ~0.73 ms | ~2.55 ms | **~1.8 ms — under the 2 ms target** |
| p99 (reported) | ~1.1–3.4 ms | ~4.3–5.9 ms | ~2.5–4.6 ms (contention-dominated) |
| throughput | ~55–87k rps | ~20–24k rps | — |

The **p50 added latency is a stable ~1.8 ms** across runs and is the asserted
baseline. The **p99 is contention-dominated in-process; see the `oha`
maintainer gate** — it is reported only, not asserted. Four back-to-back runs
gave p99 deltas of 2.48 / 3.24 / 4.57 / 3.10 ms (high run-to-run variance)
while p50 held ~1.8 ms.

Why the p99 is not the bound here: this is an *in-process, closed-loop*
measurement — the load generator, the proxy and the TLS upstream all share one
4-worker Tokio runtime on an 8-core box, driven back-to-back (max-rate), and
the `TestSink` records synchronously under a single mutex (full 16 KiB body
clones), unlike the production async queue→writer. So the proxy path saturates
the machine and its p99 reflects in-process queueing and lock contention, not
intrinsic per-request latency. The **authoritative §13 p99 < 2 ms added-latency**
figure is a maintainer-run `oha`-vs-direct measurement against a standalone
`crosswire serve` (separate-process load generator, real async writer) on the
reference machines, wired in a later task (Task 5).

# Proxy throughput gate (spec §13)

The proxy must sustain **>= 5,000 req/s** with MITM TLS on and recording on,
measured by `crates/crosswire-proxy/tests/perf_throughput.rs` with the same
in-process harness (`drive_mitm`): 64 keep-alive client-TLS connections issuing
back-to-back `GET /` for a 16 KiB body through a `CONNECT` tunnel for 3 s, which
saturates the proxy. Always run **`--release`** (debug TLS crypto is ~20x
slower). Run the two tests serially (`--test-threads=1`) so they do not contend
for the same cores:

```sh
# Report-only (CI smoke): prints the SUMMARY rows, never asserts throughput.
cargo test --release -p crosswire-proxy --test perf_throughput -- --nocapture --test-threads=1
# Maintainer gate: additionally asserts MITM >= 5000 req/s.
CROSSWIRE_PERF_ASSERT=1 \
  cargo test --release -p crosswire-proxy --test perf_throughput -- --nocapture --test-threads=1
```

| row | what | asserted |
|---|---|---|
| `throughput_mitm_at_least_5k` | MITM + recording on (`recorded > 0` always asserted) | `>= 5000 req/s` under `CROSSWIRE_PERF_ASSERT=1` |
| `throughput_sni_tunnel` | SNI passthrough tunnel, no cert minted (reference) | no |

## Reference machine: MacBook Air (Mac14,2), Apple M2, 8 cores, 8 GiB, macOS

Measured 2026-10-28, release profile, 64 connections, 3 s.

| row | req/s | p50 | p99 |
|---|---|---|---|
| MITM + recording | ~25,200 | ~2.4 ms | ~3.7 ms |
| SNI passthrough tunnel (reference) | ~43,200 | ~1.4 ms | ~2.1 ms |

Caveat: load generator, proxy and upstream share one 4-worker runtime and the
`TestSink` records synchronously, so these in-process figures are a
conservative floor; the authoritative throughput is the maintainer `oha` run
against a standalone `crosswire serve` (Task 5).

# Store ingest gate (spec §13)

The store must sustain **>= 2,000 exchanges/s** committed through the real
writer path at the §13 body mix, measured by
`crates/crosswire-store/tests/perf_ingest.rs`. It submits through the same
`ExchangeSink::submit` the proxy uses (bounded queue, then the OS writer
thread, default config: `BEGIN IMMEDIATE`, commit every 250 ms / 1,000
exchanges), not the synthetic bulk insert. Bodies follow the Plan 6 mix
(60 % < 4 KiB, 30 % 4-64 KiB, 9 % 64 KiB-1 MiB, 1 % 1-8 MiB), unique and
incompressible; the test asserts the submitted proportions are within
tolerance. The rate counts **committed** exchanges only: `Event::Committed`
ids are counted by a subscriber (also used as flow control so the queue never
overruns into the journal-spill path: at most 9,000 exchanges / 56 MiB in
flight), the writer is drained, the total is cross-checked against
`count(*) ... state = 'committed'` in the database, and the rate is committed
over the wall time from the first submit to the last commit (3 s submit
window plus the final drain). Always **`--release`**:

```sh
# Report-only (CI smoke): prints the SUMMARY line, never asserts the rate.
cargo test --release -p crosswire-store --test perf_ingest -- --nocapture
# CROSSWIRE_PERF_ASSERT=1 changes nothing here: the rate is never asserted
# against 2,000/s. Only the body mix and a gross-regression floor are asserted.
```

**Status: REPORT-ONLY.** The measured ingest rate (about 700 exchanges/s,
about 65 MB/s of body bytes, on the M2 below) is **below the §13 target of
2,000 exchanges/s**. The test prints the `SUMMARY ingest` line (committed,
secs, rate, mb_per_s, mix) and asserts only the submitted body mix and a
generous gross-regression floor of 300 exchanges/s (release builds), which is
not the §13 target. The gap is tracked in the pre-0.1.0 backlog pending a
maintainer decision: optimise the writer (parallel hash/compress, blake3, or a
lower zstd level), OR redefine the §13 ingest metric as MB/s or a lighter mix,
OR confirm on the reference 4-vCPU Linux VM.

## Reference machine: MacBook Air (Mac14,2), Apple M2, 8 cores, 8 GiB, macOS

Measured 2026-10-29, release profile, three runs.

| metric | value |
|---|---|
| committed exchanges / s | ~700 (698 / 708 / 722) |
| body bytes ingested | ~65 MB/s |
| §13 target | 2,000 / s (**not met at this mix on this machine; report-only**) |

The rate is body-volume bound, not per-exchange bound: the mean exchange at
this mix carries ~100 KB (the 9 % and 1 % buckets dominate), so 2,000/s would
need ~200 MB/s through the single writer thread (SHA-256, zstd level 3, pack
append, `fsync`). The same harness with bodies capped at 65 KiB commits ~2,400
exchanges/s. The disk is not the limit (300 MB written with a full fsync
sustains ~350 MB/s here). The authoritative figure is the maintainer run on
the reference machines.
