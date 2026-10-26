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

Measured 2026-10-27, seed 7, release profile.

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

| query | kind | warm p95 (ms) | cold p95 (ms) |
|---|---|---|---|
| host | structured | 8.8 | 3.2 |
| status>=500 and POST | structured | **12188** | **13208** |
| min_resp_len=1048577 | structured | 49.0 | 32.8 |
| tag=Y | structured | 14.5 | 11.7 |
| in-scope | structured | 4.1 | 2.6 |
| path_contains | path scan | 1.6 | 4.5 |
| path_regex | path scan | 2.9 | 10.9 |

Scroll (10 pages of 200): 1.9 ms. Peak anonymous RSS 39 MB (bound 500 MB).
Load 183.6 s; the database is about 5 GB.

**Known miss at 1 000 000:** `status>=500 and POST` takes about 12 s. The plan
is `SEARCH ... USING INDEX idx_exchanges_method_status` plus
`USE TEMP B-TREE FOR ORDER BY`: the index cannot deliver `ulid` order across
the status range, so all ~12 300 matches are fetched from the `WITHOUT ROWID`
primary b-tree (rows carry the ~5 KB `json` inline) before sorting and
`LIMIT`. Counting the same matches from the index alone takes 1.6 ms, so the
cost is the row fetches. At 100 000 rows it fits in cache (17 ms). This is an
index-design issue to fix before the 0.1.0 gate, not a probe or bench issue.
`min_resp_len` at 49 ms is also at the edge of the 50 ms target.

The 1 000 000 numbers are a single local run; the maintainer's full-gate run
on the reference machine replaces them.
