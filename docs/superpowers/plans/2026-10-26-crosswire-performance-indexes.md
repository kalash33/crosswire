# Crosswire Performance + History Indexes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the history store fast and memory-bounded at 1M exchanges — the §9 secondary indexes derived from the named queries, a normalized tag index, the missing size/path filters, and a synthetic + benchmark harness that proves the §13 p95 and RSS targets.

**Architecture:** Add SQLite secondary indexes (filter-column + `ulid`, so the keyset pages inside one index) and a normalized `exchange_tags` side table to the existing `crosswire-store`, each introduced by a `schema.rs` migration step that advances `user_version` and backfills. Extend `ExchangeFilter`/`list_query` with the size and path predicates the named queries need (size indexed; path by a bounded, row-capped scan — FTS is v0.2a). A deterministic 1M generator + a criterion bench + a cross-platform anonymous-RSS probe prove the targets; a scaled CI perf-smoke guards against regression.

**Tech Stack:** Rust, rusqlite/SQLite, criterion, the `regex` crate (already a workspace dep via `crosswire-scope`), `/proc/self/status` (Linux) + a maintained RSS crate for macOS `phys_footprint`.

**Spec:** `docs/superpowers/specs/2026-10-26-crosswire-performance-indexes-design.md`. Binding product authority: `docs/superpowers/specs/2026-10-22-crosswire-design.md` (§9 storage, §13 named queries + performance budgets). Plans 2–5b are merged on `main`.

## Current state (read before starting)
- `crates/crosswire-store/src/read.rs`: `ExchangeFilter { method, host, host_contains, status: Option<RangeInclusive<u16>>, namespace, state, scope_state, tag }` (NO size or path field); `LIST_COLUMNS` = `SELECT ulid, created_at, req_method, req_host, req_path, resp_status, resp_len, state, tags FROM exchanges`; `list_query` builds `conds` then `ORDER BY ulid DESC LIMIT ?` with a `next_cursor: Option<Ulid>` keyset; the tag filter is `tags <> '' AND EXISTS (SELECT 1 FROM json_each(tags) WHERE value = ?)` (full scan); `READER_POOL_MAX = 4`, cache 16 MiB, `mmap_size` 256 MiB.
- `crates/crosswire-store/src/schema.rs`: `init` stamps `application_id` + `user_version = USER_VERSION` in one tx; `migrate` runs each step in its own tx that advances `user_version` (an existing step bumps to 2). **No secondary indexes exist** (only the `ulid` PK).
- `crates/crosswire-store/src/writer.rs:897`: `INSERT INTO exchanges (ulid, namespace, state, scope_state, created_at, json, req_method, req_host, req_path, resp_status, resp_len, tags, notes)` — the ingest point. `crates/crosswire-store/src/control.rs::exchange_set_tags` — the tags-patch point.

## Global Constraints

- **Rust:** `rust-version = "1.88"`, `unsafe_code = "forbid"` (the macOS RSS probe must get `phys_footprint` from a crate that encapsulates the unsafe — we write none), `missing_docs = "deny"`; rustls stays ring-only; `cargo deny check` clean after any new dependency; no new default features. The generator and bench live behind a cargo feature / in `benches/` so a normal `cargo build`/`cargo install` does not pull them.
- **No query-semantics change:** every index and query rewrite must return the **same rows in the same order** as the pre-index scan (proven by a parity test per query). An index is a performance change only.
- **Migrations:** each schema change is a `schema.rs` migration step that advances `user_version` by one, is idempotent (`IF NOT EXISTS`), runs at store open on an older project, and backfills derived data (the tag table) completely before the version is advanced.
- **RSS metric:** the measured quantity is **anonymous** RSS — `RssAnon` (Linux `/proc/self/status`) / `phys_footprint` (macOS) — which excludes the mmap'd DB file by design; never `VmRSS`/physical-including-file.
- **Targets (v4 §13):** at 1M synthetic exchanges, structured named queries (host; status≥500 ∧ method=POST; size>1 MiB; tag=Y; in-scope) **p95 warm < 50 ms, cold < 500 ms**; the path queries (regex, text-in-path) are the bounded-scan exception (row cap, no p95 assertion until FTS); anonymous RSS **< 500 MB** at peak; body-size mix 60 % <4 KiB / 30 % 4–64 KiB / 9 % 64 KiB–1 MiB / 1 % >1 MiB.
- **Commits:** Conventional Commits, ≤72-char subject, a real why-body, author Kalash Poddar <kalashpoddar33@gmail.com>, and **no attribution trailers**. Verify `git log -1 | grep -c -i claude` is 0 after each commit. Run `just lint` (clippy + `cargo fmt --check`), `just doc`, and `cargo deny check` — not only `cargo test` — before considering a task done (see [[finish-verification-full-gate]]).

## Review Focus

- **An index silently changes query results** (returns different or differently-ordered rows than the pre-index scan): the structured query results must be byte-identical to the scan — Task 0/1/2 parity tests.
- **Keyset pagination under a filter index drops or duplicates a row across pages** (the `ulid` cursor interacting with the filter index): paging a filtered result to exhaustion yields every matching row exactly once, in order — Task 0 test.
- **The tag migration on an existing project leaves the tag table un-backfilled** (tag=Y returns nothing for rows written before the migration): a pre-migration tagged project returns the right rows after open — Task 1 backfill test.
- **A hostile or invalid path regex panics, hangs, or scans unbounded** (ReDoS / bad pattern): an invalid regex is rejected with a typed error, and the scan is row-capped — Task 2 tests.
- **The RSS probe measures `VmRSS` (including the mmap'd DB) instead of `RssAnon`**, making the 500 MB gate meaningless: the probe returns anonymous memory and ignores a large file-backed mmap — Task 4 test.

---

### Task 0: Structured secondary indexes, the size filter, and the migration

**Files:**
- Modify: `crates/crosswire-store/src/schema.rs` (a migration step advancing `user_version` that `CREATE INDEX IF NOT EXISTS` the four indexes)
- Modify: `crates/crosswire-store/src/read.rs` (`ExchangeFilter` gains a size predicate; `list_query` adds its `conds` clause)
- Test: `crates/crosswire-store/tests/indexes.rs` (new)

**Interfaces:**
- Produces: four indexes — `idx_exchanges_host (req_host, ulid)`, `idx_exchanges_method_status (req_method, resp_status, ulid)`, `idx_exchanges_resp_len (resp_len, ulid)`, `idx_exchanges_scope (scope_state, ulid)`. `ExchangeFilter` gains `pub min_resp_len: Option<u64>` (query 5, "size > 1 MiB" → `resp_len > 1_048_576`; a `>` filter on the column, which the `resp_len` index serves). `list_query` emits `resp_len > ?` when set. A helper `fn explain_query_plan(conn, filter) -> Vec<String>` (test-only, in the test or a `#[cfg(test)]` store method) returns the `EXPLAIN QUERY PLAN` rows for a filter so tests can assert index usage.

- [ ] **Step 1: Write the failing tests** — `indexes.rs`: for each of host / (status≥500 ∧ method=POST) / (min_resp_len) / scope filters, seed a project, run the filter, and assert (a) the result equals the same filter run with the indexes dropped (parity), and (b) `EXPLAIN QUERY PLAN` contains `USING INDEX idx_exchanges_…` and not `SCAN exchanges`. A paging test: seed N>page rows matching a host filter, page via `next_cursor` to exhaustion, assert every matching ulid appears exactly once, descending.
- [ ] **Step 2: Run** `cargo test -p crosswire-store --test indexes` → FAIL (no `min_resp_len`, no indexes).
- [ ] **Step 3: Implement** the migration step (the four `CREATE INDEX IF NOT EXISTS`, advancing `user_version`), add `min_resp_len` to `ExchangeFilter` + its `conds` clause in `list_query`.
- [ ] **Step 4: Run** `cargo test -p crosswire-store` → PASS; `just lint` + `just doc` + `cargo deny check` clean.
- [ ] **Step 5: Commit** `feat(store): index host, status+method, size and scope queries`.

---

### Task 1: Normalized `exchange_tags` side table

**Files:**
- Modify: `crates/crosswire-store/src/schema.rs` (next migration step: `CREATE TABLE exchange_tags` + backfill from existing `tags` JSON)
- Modify: `crates/crosswire-store/src/writer.rs:897` area (insert tag rows alongside the exchange insert, same tx)
- Modify: `crates/crosswire-store/src/control.rs::exchange_set_tags` (replace the tag rows for the exchange)
- Modify: `crates/crosswire-store/src/read.rs` (rewrite the tag filter to use the side table)
- Test: `crates/crosswire-store/tests/tags_index.rs` (new); extend the Task 0 EQP helper

**Interfaces:**
- Consumes: the Task 0 EQP helper and parity pattern.
- Produces: `CREATE TABLE exchange_tags (ulid TEXT NOT NULL, tag TEXT NOT NULL, PRIMARY KEY (tag, ulid)) WITHOUT ROWID;`. The tag filter in `list_query` becomes a join/`IN` against `exchange_tags` keyed by `tag` and ordered by `ulid DESC` (the implementer picks the join vs `IN (SELECT ulid …)` form that `EXPLAIN QUERY PLAN` shows using the PK index; prove it). The JSON `tags` column stays (row display + `Store::get`); writers keep both consistent in one transaction.

- [ ] **Step 1: Write the failing tests** — `tags_index.rs`: (a) parity — `tag = Y` returns the same rows as the old `json_each` scan on a seeded project; (b) EQP — the tag query uses the `exchange_tags` PK index, not `SCAN exchanges`; (c) backfill — open a project created at the pre-tag-table `user_version` with seeded tagged rows, then assert `tag = Y` returns them (the migration backfilled); (d) consistency — `exchange_set_tags` to a new tag makes `tag = new` match and `tag = old` not match.
- [ ] **Step 2: Run** `cargo test -p crosswire-store --test tags_index` → FAIL.
- [ ] **Step 3: Implement** the migration (create + backfill, chunked, in the version-advancing tx), the ingest + patch maintenance, and the `list_query` rewrite.
- [ ] **Step 4: Run** `cargo test -p crosswire-store` → PASS; `just lint`/`just doc`/`cargo deny` clean.
- [ ] **Step 5: Commit** `feat(store): normalized tag index for the tag=Y history query`.

---

### Task 2: Path filters — bounded, row-capped scan

**Files:**
- Modify: `crates/crosswire-store/src/read.rs` (`ExchangeFilter` gains path predicates; `list_query` / `list` apply them as a capped scan; a typed error for a bad regex)
- Test: `crates/crosswire-store/tests/path_filter.rs` (new)

**Interfaces:**
- Produces: `ExchangeFilter` gains `pub path_contains: Option<String>` (query 8, case-insensitive substring over `req_path`) and `pub path_regex: Option<String>` (query 4). `path_contains` emits `instr(lower(req_path), lower(?)) > 0` in SQL (bounded by the existing `LIMIT`/keyset, over the candidate set narrowed by any other indexed predicate). `path_regex` compiles the pattern with the `regex` crate (reject an invalid pattern with `StoreError::` a new `BadPattern` variant, or the existing input-error variant) and filters candidate rows in Rust, reading at most a **row cap** (reuse the §13 search cap, `10_000`) of candidate rows; the returned `Page` carries a `pub capped: bool` set when the cap was hit before the page filled. (Add `capped` to `Page<T>` with `#[serde(default)]`; existing callers ignore it.)
- Consumes: Task 0's indexed predicates (a path filter combined with host/status uses the index to narrow candidates first).

- [ ] **Step 1: Write the failing tests** — `path_filter.rs`: `path_contains` returns rows whose `req_path` contains the substring (case-insensitive) and no others; `path_regex` returns rows matching the compiled pattern; an invalid regex (`"("`) returns the typed error, never a panic; with more than the cap of candidates, `capped` is true and the result holds at most the cap/page; a path filter combined with `host` returns the intersection.
- [ ] **Step 2: Run** `cargo test -p crosswire-store --test path_filter` → FAIL.
- [ ] **Step 3: Implement** the two predicates, the regex compile/validate + cap, and `Page.capped`.
- [ ] **Step 4: Run** `cargo test -p crosswire-store` → PASS; `just lint`/`just doc`/`cargo deny` clean.
- [ ] **Step 5: Commit** `feat(store): bounded path-contains and path-regex history filters`.

---

### Task 3: Deterministic 1M synthetic generator

**Files:**
- Create: `crates/crosswire-store/src/synthetic.rs` (behind a `synthetic` cargo feature, re-exported for tests/benches)
- Modify: `crates/crosswire-store/Cargo.toml` (the `synthetic` feature; a seeded-RNG dep if not already present — prefer an existing workspace dep)
- Test: `crates/crosswire-store/tests/synthetic.rs` (new, `#[cfg(feature = "synthetic")]`)

**Interfaces:**
- Produces: `pub fn generate(store: &Store, n: usize, seed: u64)` — writes `n` exchanges through the store's commit path (or a documented fast bulk-insert producing the identical on-disk column shape), deterministically from `seed`, with: a spread of `req_host` (a fixed vocabulary), `req_method` (incl. POST), `resp_status` (incl. a known fraction ≥ 500), `req_path` (varied, a known substring for the path query), `resp_len` following the §13 body-size mix (60/30/9/1 %, incl. a known fraction > 1 MiB), `scope_state` (a known in-scope fraction), and `tags` (a vocabulary where a known tag Y matches a known subset).
- Consumes: the store write path; Tasks 0–2's filter columns (the generated spread must make each named query match a non-empty, known-size subset).

- [ ] **Step 1: Write the failing tests** — `synthetic.rs`: `generate(store, 10_000, 7)` twice into fresh stores yields identical row counts and identical per-predicate match counts (determinism); the `resp_len` distribution is within tolerance of 60/30/9/1 %; each named-query predicate (host X, status≥500∧POST, size>1 MiB, tag Y, in-scope, path substring) matches a non-empty subset.
- [ ] **Step 2: Run** `cargo test -p crosswire-store --features synthetic --test synthetic` → FAIL.
- [ ] **Step 3: Implement** `generate` and the feature.
- [ ] **Step 4: Run** → PASS; confirm a default `cargo build -p crosswire-store` (no feature) does not compile `synthetic.rs`; `just lint`/`cargo deny` clean.
- [ ] **Step 5: Commit** `feat(store): deterministic synthetic exchange generator behind a feature`.

---

### Task 4: Anonymous-RSS probe + criterion bench + baseline

**Files:**
- Create: `crates/crosswire-store/src/rss.rs` (the anonymous-RSS probe), `crates/crosswire-store/benches/history.rs` (criterion), `bench/README.md` (the stored baseline; create the `bench/` dir if absent)
- Modify: `crates/crosswire-store/Cargo.toml` (`[[bench]]`, the macOS RSS crate as a `dev`/bench dependency; `criterion` dev-dep if absent), `justfile` (the `bench` recipe already builds `-p crosswire-h1`; extend it or add the store bench build)
- Test: `crates/crosswire-store/tests/rss.rs` (new)

**Interfaces:**
- Consumes: Task 3 `generate`, Tasks 0–2 filters.
- Produces: `pub fn anon_rss_bytes() -> Option<u64>` — on Linux parses the `RssAnon:` line of `/proc/self/status`; on macOS returns `phys_footprint` via a maintained crate that encapsulates the `task_info` call (we write no `unsafe`); returns `None` on unsupported targets. The criterion bench `history.rs` loads `generate(store, N, seed)` (N configurable; default a bench-sized value, 1M for the full gate), then times each named query (warm: the `LIMIT`-bounded `list` repeated on a warmed connection; cold: a fresh pooled connection with dropped cache) and records peak `anon_rss_bytes()` across load + queries + a scroll (paging the last-200 to several pages). It asserts/report: structured queries p95 warm < 50 ms, cold < 500 ms; `anon_rss_bytes()` < 500 MB at 1M; the path queries report timing + assert `capped`/the RSS bound only.

- [ ] **Step 1: Write the failing tests** — `rss.rs`: `anon_rss_bytes()` returns `Some(x)` with `x > 0` on Linux/macOS; after allocating and touching a large anonymous buffer, the value rises by roughly the buffer size (it tracks anonymous memory); mmap'ing a large file does **not** inflate it comparably (it excludes file-backed memory) — or, where a portable file-mmap check is impractical, assert the Linux reader parses the `RssAnon` line specifically (not `VmRSS`).
- [ ] **Step 2: Run** `cargo test -p crosswire-store --test rss` → FAIL.
- [ ] **Step 3: Implement** `rss.rs`, the bench, and write the baseline numbers into `bench/README.md` (per reference machine, dated) from a local run at a representative N.
- [ ] **Step 4: Run** `cargo test -p crosswire-store` → PASS; `cargo bench -p crosswire-store --no-run` builds; `cargo deny check` clean (the macOS RSS crate introduces no advisory/duplicate-license issue).
- [ ] **Step 5: Commit** `feat(store): anonymous-RSS probe and the named-query criterion bench`.

---

### Task 5: CI perf-smoke + maintainer `just perf`

**Files:**
- Modify: `justfile` (add `perf` — the full 1M RSS + named-query gate — and `perf-smoke` — a scaled run), `.github/workflows/ci.yml` (a ubuntu-gated `perf-smoke` step)
- Test: the `perf-smoke` recipe is the test (fails on a regression; passes on the current tree)

**Interfaces:**
- Consumes: Task 3 `generate`, Task 4 bench + RSS probe, Tasks 0–2 EQP assertions.
- Produces: `just perf` runs the full 1M gate (structured p95 warm<50/cold<500, RSS<500 MB) and prints a comparison to `bench/README.md`; `just perf-smoke` runs a scaled gate (≈100k rows, proportionally looser time/RSS bounds) PLUS the EQP index-usage assertions (from Tasks 0–2, which are O(1) and hardware-independent) as the regression guard. CI runs `perf-smoke` on the ubuntu `check` job only, with the `synthetic` feature enabled; it must not run on macos (kept Node/perf-free there) and must not assert the 1M numbers (CI hardware ≠ reference machines).

- [ ] **Step 1: Write the failing check** — add `just perf-smoke`; prove it FAILS when an index is dropped (an EQP assertion flips to `SCAN`) and PASSES on the current tree.
- [ ] **Step 2: Run** `just perf-smoke` against a deliberately index-dropped build → FAIL; against the current tree → PASS.
- [ ] **Step 3: Implement** the recipes and the ubuntu-gated CI step (pin any new action by SHA; keep `cargo build`/`just test` unchanged).
- [ ] **Step 4: Run** `just perf-smoke` locally → green; confirm the CI step is ubuntu-only and uses the `synthetic` feature.
- [ ] **Step 5: Commit** `ci: scaled perf-smoke regression guard plus the maintainer perf gate`.

---
