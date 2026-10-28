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
upstream (`p99_direct`) and once through a real MITM proxy that mints a leaf,
relays and records (`p99_proxy`); the per-connection TLS handshake and
`CONNECT` setup (plus a 32-request warmup) are excluded from the samples. The
gate value is `p99_proxy − p99_direct`; target `< 2 ms`. Always built
**`--release`** — debug rustls/ring crypto is ~20x slower and meaningless here.

## Reference machine: MacBook Air (Mac14,2), Apple M2, 8 cores, 8 GiB, macOS

Measured 2026-10-28, release profile, 2 s per path. **OVER the 2 ms target at
p99** (see the caveat below); recorded here as the measured baseline, not an
accepted pass.

| metric | direct | via MITM proxy | added (proxy − direct) |
|---|---|---|---|
| p50 | ~0.73 ms | ~2.55 ms | **~1.8 ms** |
| p99 | ~1.1–3.4 ms | ~4.3–5.9 ms | **~2.5–4.6 ms** |
| throughput | ~55–87k rps | ~20–24k rps | — |

Four back-to-back runs gave p99 deltas of 2.48 / 3.24 / 4.57 / 3.10 ms. The
**median** added latency is a stable ~1.8 ms (under target); the p99 delta is
repeatably over 2 ms with high run-to-run variance.

Caveat — this is an *in-process, closed-loop* measurement: the load generator,
the proxy and the TLS upstream all share one 4-worker Tokio runtime on an
8-core box, and the drive is back-to-back (max-rate), so the proxy path (≈2x
TLS crypto + synchronous mutex-guarded recording of full 16 KiB bodies)
saturates the machine and the p99 reflects queueing under saturation, not the
proxy's intrinsic per-request service time. Whether the p99 overage is a
genuine product cost or a harness artifact is unresolved; the maintainer gate
asserts the reference number so a future run can pin it down.
