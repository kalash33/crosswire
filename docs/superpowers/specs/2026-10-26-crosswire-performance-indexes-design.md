# Crosswire performance + history indexes: design spec

**Status:** design, 2026-10-26. Companion to the v4 product spec
(`docs/superpowers/specs/2026-10-22-crosswire-design.md`); that document is the
binding authority for the storage model (§9), the named history queries and
the performance budgets (§13). This spec pins **how** the store meets the
alpha.2 performance mini-DoD that §13 leaves open: the §9 secondary indexes
derived from the named queries, a bounded-RSS strategy and its measurement,
and a synthetic-data + benchmark harness.

## 1. What this is

Plan 6 makes the already-shipped recorder/store/history stack **fast and
memory-bounded at scale**, so alpha.2's performance mini-DoD is met: at
**1,000,000 synthetic exchanges**, the named history queries hit their p95
targets and anonymous RSS stays under 500 MB. It adds no new product surface
and no new API — it adds indexes, a measurement harness, and the benchmarks
that prove the targets, on the existing `crosswire-store` and the Plan 4
history API (`GET /exchanges`, `/counts`).

### 1.1 Scope

**In scope (this plan):**
- The §9 secondary indexes for the structured named queries, plus a
  normalized tag index (replacing today's `json_each` scan).
- `EXPLAIN QUERY PLAN` assertions that each named query uses its index.
- A bounded-RSS strategy and a cross-platform `RssAnon`/`phys_footprint`
  probe.
- A deterministic 1M synthetic-exchange generator (the §13 body-size mix).
- A criterion benchmark for the named queries + the RSS measurement, with a
  stored baseline; a scaled CI perf-smoke and a maintainer `just perf` full
  gate.
- The schema migration (new indexes + tag table + backfill) on an existing
  project.

**Out of scope (measured elsewhere, not this plan):**
- Proxy-path latency (`p99(proxy) − p99(direct) < 2 ms`), throughput
  (`≥ 5k req/s`) and ingest rate (`≥ 2k exchanges/s`) — the proxy/recorder
  write path owns these.
- The UI frame budget (`no frame > 32 ms over a 10 s scroll`,
  `filter applied < 300 ms`) — the Plan 5 UI owns this.
- FTS5 and fast substring/regex path search — v0.2a per §9.5; here
  `path matches regex` and `text in path` are served by a bounded,
  row-capped scan.

### 1.2 Binding targets (from v4 §13, verbatim intent)

At **1M synthetic exchanges**, on both reference machines (an Apple M-series
laptop and a 4-vCPU Linux VM):
- **Anonymous RSS** (`RssAnon` on Linux / `phys_footprint` on macOS)
  **< 500 MB** at peak, measured after loading 1M exchanges and running all
  named queries and a scroll.
- **Named history queries** at **p95 warm < 50 ms, cold < 500 ms**:
  (1) last 200 by ULID; (2) host = X; (3) status ≥ 500 ∧ method = POST;
  (4) path matches regex; (5) size > 1 MiB; (6) tag = Y; (7) in-scope only;
  (8) text in path. Queries (4) and (8) are the bounded-scan exception
  (§3.4): they carry a row cap and are documented as not meeting the 50 ms
  warm target at 1M until FTS (v0.2a); the structured queries (1,2,3,5,6,7)
  meet it.
- The synthetic body-size mix: 60 % < 4 KiB, 30 % 4–64 KiB, 9 % 64 KiB–1 MiB,
  1 % > 1 MiB.

## 2. Current state (what Plan 6 builds on)

- `exchanges` has denormalized columns — `ulid` (PK), `created_at`,
  `req_method`, `req_host`, `req_path`, `resp_status`, `resp_len`, `state`,
  `scope_state`, `tags` (a JSON-array string) — alongside the full `json`
  blob. The list query (`crosswire-store/src/read.rs`) is
  `SELECT <cols> FROM exchanges WHERE <conds> ORDER BY ulid DESC LIMIT ?`
  with a ULID keyset cursor (`next_cursor`).
- **No secondary indexes** exist beyond the `ulid` primary key. The tag
  filter runs `tags <> '' AND EXISTS (SELECT 1 FROM json_each(tags) WHERE
  value = ?)` — a full scan (read.rs notes it "scans rather than using an
  index").
- The read pool is bounded: `READER_POOL_MAX = 4` connections, each
  `PRAGMA cache_size = -16384` (16 MiB) and `mmap_size = 268435456` (256 MiB,
  file-backed → excluded from `RssAnon`). Schema version is tracked via
  `PRAGMA user_version`.

So: `last-200-by-ULID` already rides the PK keyset; the gap is the other
structured queries (no indexes) and the tag query (full scan).

## 3. Design

### 3.1 Index set (derived from the named queries)

Create these secondary indexes on `exchanges`, each trailing `ulid` so the
`ORDER BY ulid DESC` keyset pages inside the same index (filter + order from
one index; the `LIMIT`-bounded result means a per-row PK lookup for the few
display rows is cheap, so no covering-index bloat):

- `idx_exchanges_host` on `(req_host, ulid)` — query (2) host = X.
- `idx_exchanges_method_status` on `(req_method, resp_status, ulid)` —
  query (3) status ≥ 500 ∧ method = POST (equality on method, range on
  status, order by ulid).
- `idx_exchanges_resp_len` on `(resp_len, ulid)` — query (5) size > 1 MiB.
- `idx_exchanges_scope` on `(scope_state, ulid)` — query (7) in-scope only.

`created_at` is **not** separately indexed: the list orders by `ulid`
(monotonic with time), and `last-200` uses the PK. The implementer confirms
SQLite's planner chooses the ULID direction for keyset paging under each
filter (via the EQP assertions, §3.5).

### 3.2 Tag query — a normalized side table

Replace the `json_each` scan with a side table, maintained wherever the
denormalized `tags` column is written (ingest and the tags-patch path in
`control.rs::exchange_set_tags`):

```
CREATE TABLE exchange_tags (ulid TEXT NOT NULL, tag TEXT NOT NULL,
                            PRIMARY KEY (tag, ulid)) WITHOUT ROWID;
```

Query (6) becomes
`... JOIN exchange_tags t ON t.ulid = exchanges.ulid WHERE t.tag = ? ORDER BY exchanges.ulid DESC LIMIT ?`
(or an `IN (SELECT ulid FROM exchange_tags WHERE tag = ? ...)` keyset form —
the implementer picks whichever the planner serves fastest, proven by EQP).
The `(tag, ulid)` PK gives a tag → ULIDs range scan ordered for the keyset.
The JSON `tags` column stays (it feeds the row display and `Store::get`);
the side table is the index. Writers keep them consistent in the same
transaction.

### 3.3 RSS strategy & measurement

`RssAnon` (Linux `/proc/self/status`) / `phys_footprint` (macOS
`task_vm_info`/`libproc`) count **anonymous** memory — the SQLite page cache
and the app's own allocations — **not** the mmap'd DB file holding the 1M
rows. So the budget is governed by: the read pool (≤ 4 conns) × `cache_size`
(16 MiB) = ≤ 64 MiB of page cache; keyset pagination capping any result to
`PAGE_LIMIT_MAX` (1000) rows so no query materializes the table; and the
in-flight body budget already shared by proxy/intercept/repeater. The new
indexes add **disk**, not anonymous RSS. Design constraints the plan must
hold: the pool and `cache_size` stay bounded; no named query or the scroll
path buffers more than a page of rows; the generator's bulk load does not
leave a large cache resident at measurement time (checkpoint/close between
load and measure). A small `crosswire-store` helper exposes the current
anonymous RSS for the harness (`#[cfg(target_os)]`-gated: `RssAnon` on Linux,
`phys_footprint` on macOS; documented unsupported elsewhere).

### 3.4 Path-regex / text-in-path — bounded scan

Queries (4) and (8) cannot use a B-tree index and FTS is deferred (§9.5). They
run as a scan over the (indexed-where-possible) candidate set with an explicit
row cap (reuse/define the §13 search cap, 10k) and a statement timeout; the
result carries a "capped" indicator. These are documented as the v0.2a FTS
upgrade and are **excluded from the p95 < 50 ms warm assertion** at 1M (they
still carry a cold-path budget and must not blow the RSS bound — the cap
bounds buffered rows).

### 3.5 EXPLAIN QUERY PLAN assertions

For each structured named query (2,3,5,6,7), a test runs `EXPLAIN QUERY PLAN`
and asserts the plan uses the intended index (e.g. `SEARCH exchanges USING
INDEX idx_exchanges_host`) and **not** `SCAN exchanges`. This pins the
performance contract in a fast, deterministic test independent of timing. The
last-200 query asserts the PK/ULID keyset path; the tag query asserts the
`exchange_tags` index.

### 3.6 Schema migration

Bump `PRAGMA user_version`; in the migration: `CREATE INDEX` the four indexes,
`CREATE TABLE exchange_tags`, and **backfill** `exchange_tags` from existing
rows' `tags` JSON (chunked, inside a transaction). The migration is
idempotent (`IF NOT EXISTS`) and runs at store open on an older project. A
test opens a pre-migration project with seeded tagged rows and asserts the
tag query returns the same rows after migration.

### 3.7 Synthetic generator

A deterministic generator (seeded RNG) that bulk-inserts N exchanges with the
§13 body-size mix and realistic variety across the indexed columns (a spread
of hosts, methods, statuses incl. ≥ 500, resp_len incl. > 1 MiB, scope
states, and a tag vocabulary so tag = Y matches a known subset). It writes
through the store's commit path (or a documented fast bulk path that produces
an identical on-disk shape) so the measured DB equals a real one. Lives behind
a test/bench feature or a `crosswire` dev subcommand; not in the shipped
release surface. Deterministic so the bench is reproducible.

### 3.8 Benchmark harness, baseline, and the gate

- A criterion bench (`crates/crosswire-store/benches/history.rs`, wired into
  `just bench`) times each named query (warm: repeated; cold: fresh
  connection / dropped cache) at the configured row count, and records the
  peak anonymous RSS during load + queries + a scroll.
- **Targets asserted:** structured queries p95 warm < 50 ms, cold < 500 ms;
  RSS < 500 MB at 1M. The bounded-scan queries (4,8) assert only the cap +
  the RSS bound.
- **Baseline** stored in `bench/README.md` (numbers per reference machine,
  dated), the §13 "stored baseline".
- **CI perf-smoke:** a scaled run (≈ 100k rows, proportionally looser time
  bounds, RSS bound scaled) on the ubuntu job as a **regression guard** — it
  fails if a named query stops using its index (the EQP assertions run here
  cheaply at full logic) or if timings/RSS regress past the scaled bound. It
  does **not** assert the 1M numbers (CI hardware ≠ reference machines).
- **Maintainer full gate:** `just perf` runs the full 1M-row RSS + named-query
  gate on the two reference machines; the result updates the stored baseline.

## 4. Testing

- **EQP index-usage tests** (§3.5) — one per structured named query; the
  core performance contract, deterministic.
- **Correctness parity** — for each named query, a test asserts the indexed
  result set equals the pre-index (scan) result on a seeded project (same
  rows, same order), so an index never changes semantics.
- **Migration test** (§3.6) — pre-migration project with tagged rows; after
  open, the tag query and the EQP assertions pass; backfill is complete.
- **Generator determinism** — same seed → same row shape/counts.
- **RSS probe** — returns a plausible non-zero value on Linux/macOS; the
  bench asserts the < 500 MB bound at 1M (full gate) / scaled bound (smoke).
- **CI** — the perf-smoke (EQP + scaled timings/RSS at ≈ 100k) runs on the
  ubuntu job; `just bench` builds.

## 5. Decisions on record

- Scope: storage/query/RSS only; proxy-path latency/throughput/ingest and the
  UI frame budget are measured by their own subsystems, not here.
- Structured named queries get B-tree indexes (filter-col, ulid) + a
  normalized `exchange_tags` side table; path-regex / text-in-path use a
  bounded, row-capped scan now, with FTS5 as the v0.2a upgrade.
- In-memory/custom index layers were rejected — they consume anonymous RSS
  (the budget) and reinvent what SQLite indexes give for free. No separate
  projection table — the denormalized columns already exist.
- The perf gate is a scaled CI smoke (regression guard, incl. the EQP
  assertions) plus a maintainer-run `just perf` full 1M gate on the two
  reference machines with a stored baseline.
- `RssAnon`/`phys_footprint` (anonymous memory) is the measured quantity;
  the mmap'd DB file is excluded by design, which is why these metrics were
  chosen in §13.

## 6. Deferred (named, not dropped)

- FTS5 (external-content, trigram) and fast substring/regex search over head,
  body and path — v0.2a (§9.5); path-regex / text-in-path performance rides
  it.
- Proxy-path performance (added latency, throughput, ingest rate) and the UI
  frame budget — their own subsystems' gates.
- Retention/purge performance at scale and encryption-at-rest — later (§9.3,
  §17).
