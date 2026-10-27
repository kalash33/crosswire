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
