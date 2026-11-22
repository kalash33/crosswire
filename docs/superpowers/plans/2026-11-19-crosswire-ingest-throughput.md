# P2 Ingest Throughput Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the last REPORT-ONLY §13 row: **≥ 2,000 committed exchanges/s at the §13 mix** through the real sink and writer on the M2 reference machine, asserted by `perf_ingest` under `CROSSWIRE_PERF_ASSERT=1` (median of 3 runs ≥ 2,000/s, every run ≥ 1,800/s, 10 s windows). MB/s and a lighter "typical" mix are reported, never asserted. No format change, no durability change, the target is not lowered.

**Architecture:** Measure first (T0: per-stage writer timers behind a non-default cargo feature, an in-process SHA-256 MB/s probe, a recorded baseline of the 0.1.1 writer on `sha2 0.11`). Then remove waste on the one writer thread, one commit per item (T1: hash each pack body once, stop base64-encoding bodies into a JSON value that is thrown away, `prepare_cached` for every per-exchange statement). Only if T0/T1 numbers say so, move the pure per-body work (SHA-256, zstd, row JSON) to a bounded pool of 0–4 std threads whose results the writer consumes strictly in batch order (T2). Every file, fsync, SQL statement and event stays on the writer thread in today's order.

**Tech Stack:** Rust (MSRV 1.89), `rusqlite` 0.41.1 with bundled SQLite 3.54.1, `sha2` 0.11.1 (from the dependency refresh), `zstd` bulk API, std threads and `std::sync::mpsc` (no rayon, no new crate).

**Spec:** `docs/superpowers/specs/2026-11-19-crosswire-ingest-throughput-design.md` (section numbers below refer to it). **Binding decisions:** the controller decisions D1–D10 below (kept in full here; the draft they came from is not in the repo). Where a decision and the spec differ, the decision wins; the spec text already reflects D1, D5, D6, D8 and D9:
- D1: P2 starts **after** the dependency refresh. Done: PR #35 merged as `e4e1e6d`, so `main` locks `sha2 0.11.1` (`75d02c0`), whose aarch64 SHA2 backend is selected at run time, and the known-answer tests `blob_hash_is_fips_sha256` (`crosswire-store/tests/packs.rs`) and `relay_sha256_matches_kat` (`crosswire-proxy/src/capture.rs`) already exist. The spec's §7 Q1 `sha2` work is void. If T0's probe shows the hardware SHA backend is not active on aarch64, enabling it is a T1 item (T1.0), not a reason to stay on 0.10.
- D2: gate = median of 3 ≥ 2,000/s, every run ≥ 1,800/s, 10 s windows, M2 only.
- D3/D4: the `typical` mix and the Linux VM row are REPORT-ONLY.
- D5: §13 text clarified to "1 % 1–8 MiB uniform" (docs task, T3).
- D6: no CLI flag. `StoreConfig::prepare_threads` default = `min(4, available_parallelism − 1)` (saturating, so a 1-CPU host gets 0); `0` = serial; the env override `CROSSWIRE_INGEST_PREPARE_THREADS` is read by the **harness only**, never by the library. (The spec's `clamp(ap/2, 1, 4)` is superseded.)
- D7: a body that later dedups may be compressed on a worker for nothing. Accepted.
- D8: the thread default is D6's `min(4, ap − 1)`, not the spec's `clamp(ap/2, 1, 4)` (spec §4.2.1 already updated).
- D9: a worker panic is caught on the worker; the writer re-prepares that item **once, inline on the writer thread**, sends one `Error` event and commits the batch (nothing is lost). A second panic (the inline re-prepare) unwinds the writer: today's `DeathWatch` path. This departs from the spec draft's "re-raise on the writer"; spec §4.2 and §6 already updated.
- D10: the ULID canonical-parse hardening shipped in the dependency refresh (`a8b6e1b`), not in P2.

**Task numbering differs from spec §9.** Spec T0 → T0; spec T1+T2 → T1 (hardware SHA only if the probe says so; `sha2` bump is already done by D1); spec T3 → T2; spec T5/Tn → T3 + T4. The spec's criterion micro-bench (`benches/ingest_stages.rs`) is replaced by T0's stage counters and in-test SHA-256 probe (no new bench target). Spec T4 (lower zstd level) is **not in this plan**: it is a contingency that needs a maintainer ruling (see T2 exit).

**Branch:** `feat/ingest-throughput` from `main` at `e4e1e6d` (the dependency-refresh merge). First commit: `docs: 0.1.2 ingest throughput spec and plan` (spec + this plan into `docs/superpowers/{specs,plans}/`). Every file:line below was re-checked on `e4e1e6d`.

## Global Constraints
- **No on-disk format change.** SHA-256 stays the hash everywhere (pack header, `blobs.hash`, `bodies.blob_hash`, JSON `blob.hash`). No new pack record version, no `user_version` change (stays 6), no migration, `docs/format.md` contract text unchanged (one descriptive "Write path" sentence allowed in T3). The zstd level stays 3. `compat.rs::opens_0_1_0_project` and the `tests/fixtures/project-0.1.0/` fixture are **not edited** and must pass after every task.
- **Stored `exchanges.json` text is byte-identical to 0.1.1** for the same exchange (proved by a reference-builder test in T1.2, kept through T2).
- **No change to when anything becomes durable.** Per batch: `BEGIN IMMEDIATE` → markers → bodies and rows → `last_committed_ulid`/watermark → pack `sync_all` → `committed_len` → `COMMIT` → WAL fsync → journal-file delete → `Committed`/`Marker` events. Workers do **no** file I/O, no SQL, no events, hold no connection. Batch bounds (250 ms / 1,000), queue bounds (10,000 / 64 MiB), checkpoint settings, `synchronous` and every fsync are unchanged.
- **No loosening of any perf assertion.** The §13 target, the 1,800/s per-run floor, the body-mix tolerances and every existing perf/RSS assertion stay or tighten. The gross floor only goes up (T4). `limits_rss_store.rs` passes unchanged. A run with `gen_busy_pct > 80` is **invalid**, not passing.
- No new crate in the dependency graph (`cargo deny check` clean; `cargo tree -d` unchanged by this branch). No `unsafe`. `missing_docs = "deny"` holds; new test-only/instrumentation items are `#[doc(hidden)]` and documented.
- The library never reads an environment variable for P2 behaviour. Harness knobs are env vars read in `tests/perf_ingest.rs` only.
- **Cargo commands run sequentially on the dev box, one at a time, never in parallel** (shared `target/`), never piped through `tail`: each writes to a log in the scratchpad, then read its summary and exit code. The ~40-minute workspace test runs **once**, in T4, in the background.
- Every gate measurement uses exactly this invocation (spec §5.2), on the M2, on AC power, **Low Power Mode off** (`pmset -g | grep lowpowermode` shows 0), nothing else heavy running, release build:
  ```sh
  CROSSWIRE_PERF_ASSERT=1 CROSSWIRE_INGEST_RUNS=3 CROSSWIRE_INGEST_WINDOW_SECS=10 \
    cargo test --locked --release -p crosswire-store --test perf_ingest -- --nocapture --test-threads=1
  ```
  Stage splits come from a **separate** run of the same line plus `--features stage-timing`; the asserted numbers are always from the build without the feature.
- Conventional Commits, subject ≤ 72 characters, a body that says what changed, why, and the measurement line. **NO `Co-Authored-By`, `Claude-Session` or any AI attribution.**
- Each task ends in exactly the commits it names, so each optimisation can be reverted alone.

## Review Focus
Trace each of these through the code (file:line in the review notes), not just the tests:
1. **A worker panics while preparing item 2 of a 5-item batch.** The worker catches it (`catch_unwind` around `prepare`), returns `Prepared::Panicked` for that slot and keeps serving. The writer re-prepares item 2 inline, sends one `Error` event, and commits items 1–5 in order with the same `Committed` ids as at `prepare_threads = 0`. Nothing goes to `journal/bad/`, no gap marker. Only if the inline re-prepare also panics does the writer unwind, and then it is today's `DeathWatch` path (`writer_died`, `submit` → `Closed`).
2. **Item 3 of 6 fails as its own fault on a worker (body > 64 MiB → `InvalidInput`), so it is set aside and the batch is retried.** `try_commit` returns `CommitError::Item(2, e)` only after it has drained or discarded every outstanding result of that attempt. The retry of the other 5 re-dispatches them under a new batch generation, and no stale result from the failed attempt can be consumed (the generation/index tag is checked). The pack writer is reopened at `committed_len` as today. Item 3 is in `journal/bad/`, one gap marker is written, the rest commit in submission order.
3. **Shutdown with 3 bodies in flight on workers.** On `close` (clean), `finish` commits all 3 before the pool is joined and before `close` returns. On `Drop`, all 3 are committed or written back to `journal/`. On `kill -9` between the frame `write_all` and `COMMIT`, none of the 3 is referenced by the database: the startup sweep truncates the pack past `committed_len`, and any of the 3 that came from `journal/` is replayed on the next open (its file was not deleted, because deletion follows the WAL fsync). Workers never hold the project, so the project is returned only after every worker has been joined, and the join cannot block on I/O.
4. **Byte-identical JSON.** For an exchange with a `Normalized` head, `error` state, two tags (one repeated), notes, a request body of exactly 16 KiB (inline) and a response of 16 KiB + 1 (pack) with `truncated_at`, `capture_dropped` and `relay` set, the new builder's string equals the 0.1.1 reference builder's string byte for byte, at `prepare_threads` 0 and 4; and the fixture's `stored_json_sha256` values still match in `opens_0_1_0_project`.
5. **One hash, one slice.** For a pack body, the hash written in the 40-byte record header, `blobs.hash`, `bodies.blob_hash` and the JSON `blob.hash` are the same 32 bytes, computed once from the same `&[u8]` that zstd compresses, and equal to an independent SHA-256 of the submitted bytes and to literal hex constants. On a dedup hit no frame is appended, `refcount` is bumped, and the worker's frame is dropped. A debug build re-hashes in `append` and asserts equality.

## Parallelism
```
T0 (instrument + probe + baseline) ─> T1.1 ─> T1.2 ─> T1.3 [─> T1.4 optional] ─> T2 (only if the T0/T1 rule says so) ─┐
T3 docs (no cargo; numbers filled from T0–T2 ledger) ─────────────────────────────────────────────────────────────┴─> T4 gate
```
Everything that runs cargo is strictly sequential on one branch: every T1/T2 item ends in a gate measurement, and two cargo runs on the box would corrupt both timings. T3 may be drafted in a parallel worktree (no cargo) and is finished after the last measurement.

---

### Task 0: measure before changing anything (opus)
Nothing on the writer's behaviour changes in this task. Its numbers decide whether T2 exists.

**Files:**
- `crates/crosswire-store/Cargo.toml`: new non-default feature `stage-timing = []` with a comment (instrumentation for `perf_ingest`, never enabled by the binary). `just lint`/`doc`/`test` use `--all-features`, so it is compiled and linted in CI.
- Create `crates/crosswire-store/src/stages.rs`: `#[doc(hidden)] pub struct StageTimes` (one `AtomicU64` of nanoseconds and one count per stage: `exists`, `json`, `sha_put`, `sha_append`, `sha_inline`, `zstd`, `pack_write`, `sql_bodies`, `sql_exchange`, `fsync`, `commit`, and in T2 `prepare_wait`), a `#[doc(hidden)] pub fn snapshot()`, and a `stage!(name, expr)` macro that compiles to just `expr` without the feature. `Store::stage_times()` exists only under the feature. *As built:* the counters are one process-wide static (the harness subtracts a snapshot taken at the start of each run); three stages were added so the split sums to the writer's time: `sql_batch` (`BEGIN`, markers, `meta`, `committed_len`), `batch` (the whole `try_commit`) and `after_commit`; the table prints `(other)` = `batch` minus the inner stages.
- Modify `src/writer.rs`, `src/bodies.rs`, `src/packs.rs`: wrap the stages of spec §3.3 (a–k) in `stage!`. No other edit. Under `#[cfg(not(feature = "stage-timing"))]` the expanded code is token-identical in effect.
- Modify `crates/crosswire-store/tests/perf_ingest.rs`:
  - `CROSSWIRE_INGEST_RUNS` (default 1), `CROSSWIRE_INGEST_WINDOW_SECS` (default 3), `CROSSWIRE_INGEST_MIX` = `s13` (default) | `typical` (70 % < 4 KiB, 25 % 4–64 KiB, 4.5 % 64 KiB–1 MiB, 0.5 % 1–4 MiB; half the bodies deterministic ASCII text, labelled a proposal), `CROSSWIRE_INGEST_PREPARE_THREADS` (parsed now, rejected with a clear message until T2 wires it). A fresh project per run, 5 s pause between runs.
  - SUMMARY keeps every existing field and adds `run=`, `mix=`, `prepare_threads=` (`n/a` before T2). *As built:* the existing `mix=60.1/29.9/…` shares field and the new mix name share one field, `mix=s13:60.1/29.9/9.0/1.0`; `gen_busy_pct=` (time in body fill + `Exchange` build + `submit` over the window) and, under the feature, `stage_ms=json:…,sha_put:…,…`. After the runs: `SUMMARY ingest-gate median=… min=… runs=a/b/c`.
  - A run with `gen_busy_pct > 80` fails the test as invalid (harness-bound) in every mode.
  - Rate assertion against 2,000/s is **not** added yet (T4). The mix checks and the 300/s floor are unchanged.
  - New `#[test] fn sha256_backend_throughput()`: hashes a 64 MiB buffer 16 times with `sha2::Sha256::digest` (release only; skipped in debug), prints `SUMMARY sha256 arch=… cpu_sha2=yes|no mb_per_s=… backend=hw|soft?` where the hint is `hw` when ≥ 1,000 MB/s on `aarch64` (`cpu_sha2` is `std::arch::is_aarch64_feature_detected!("sha2")`). Report-only in T0.
  - New `#[test] fn body_json_encode_throughput()` (release only): `serde_json::to_value` of an exchange with a 1 MiB inline body, printed as `SUMMARY json-encode … ms_per_mib=…`; step 4's base64 share comes from it.
- `bench/README.md`: a new "P2 baseline (0.1.1 writer)" table under "Store ingest gate" (filled in step 3).

**Tests:** `cargo test --locked -p crosswire-store --test perf_ingest` (debug: mix checks only), `cargo test --locked -p crosswire-store --all-features --lib`, `cargo clippy --locked -p crosswire-store --all-targets --all-features -- -D warnings`, then the same clippy without `--all-features`.

- [x] Step 1: implement the counters, the harness knobs and the probe (done; `nm` on the release rlib without the feature shows 0 `stage` symbols, 12 with it; the noise comparison waits for step 3). Prove the feature is free when off: `cargo build --locked --release -p crosswire-store` before and after shows no new symbol named `stage` (`nm` on the rlib is enough) and the gate rate without the feature is within run-to-run noise of step 3's.
- [ ] Step 2: run the SHA probe: `cargo test --locked --release -p crosswire-store --test perf_ingest sha256_backend_throughput -- --nocapture`. Record MB/s. **If < 1,000 MB/s on the M2, the hardware backend is not active:** T1.0 below becomes mandatory. *Smoke, not a baseline (agent box, Low Power Mode):* `arch=aarch64 cpu_sha2=yes mb_per_s=2159..2267 backend=hw`, so T1.0 is not expected; confirm from step 3's `s13.log`.
- [ ] Step 3 (maintainer): **take the baseline on the M2 with exactly these commands** (AC power, Low Power Mode off, nothing else heavy; each command alone, one after another; the agent box that built T0 runs in Low Power Mode, so its numbers are smoke, not a baseline):
  ```sh
  pmset -g | grep lowpowermode          # must print: lowpowermode 0
  L=/tmp/p2-baseline; mkdir -p $L
  CROSSWIRE_PERF_ASSERT=1 CROSSWIRE_INGEST_RUNS=3 CROSSWIRE_INGEST_WINDOW_SECS=10 \
    cargo test --locked --release -p crosswire-store --test perf_ingest -- --nocapture --test-threads=1 \
    > $L/s13.log 2>&1; echo "exit=$?"
  CROSSWIRE_PERF_ASSERT=1 CROSSWIRE_INGEST_RUNS=3 CROSSWIRE_INGEST_WINDOW_SECS=10 CROSSWIRE_INGEST_MIX=typical \
    cargo test --locked --release -p crosswire-store --test perf_ingest -- --nocapture --test-threads=1 \
    > $L/typical.log 2>&1; echo "exit=$?"
  CROSSWIRE_PERF_ASSERT=1 CROSSWIRE_INGEST_RUNS=3 CROSSWIRE_INGEST_WINDOW_SECS=10 \
    cargo test --locked --release -p crosswire-store --features stage-timing --test perf_ingest -- --nocapture --test-threads=1 \
    > $L/s13-stages.log 2>&1; echo "exit=$?"
  grep -E '^(INGEST|SUMMARY|STAGES|  )' $L/s13.log $L/typical.log $L/s13-stages.log
  ```
  The `SUMMARY sha256` line in `s13.log` is step 2's reading on the reference machine. Then record (below) and commit the baseline and the T2 decision separately from the tooling: `docs(bench): record the P2 ingest baseline`.
- [ ] Step 3 record: **baseline, before any writer change:** the gate invocation (3 × 10 s, `s13`), then `CROSSWIRE_INGEST_MIX=typical` with the same line, then the `s13` line again with `--features stage-timing`. Record in `bench/README.md` (P2 baseline table): the 3 rates, median, min, `mb_per_s`, `gen_busy_pct`, and the stage split as ms per 1,000 exchanges and as % of writer time.
- [ ] Step 4: **the T2 decision (write it into the commit body and the README table).** From the stage split compute
  `t_now` = writer ns per exchange (sum of stages), and `t_proj = t_now − sha_append − (share of json spent base64-encoding bodies, from a 1 MiB `to_value` measurement in the same test binary) − 0.7 × sql_* `prepare` overhead (estimated as half of sql_* time unless measured)`, with SHA costs scaled to the probe's hardware MB/s if T1.0 applies.
  - If `1e9 / t_proj ≥ 2,600` (30 % over the target, to cover the 1,800 per-run floor, thermal drift on a fanless M2, and estimate error): **T2 is provisionally not needed.** T1's exit measurement confirms or overturns this.
  - Otherwise **T2 is needed** and is scheduled now; T1 still lands first (parallelising wasted work is wrong).
  - **Check `writer_busy_pct` in the `STAGES` line first.** The T0 smoke (Low Power Mode, 2 s, not a baseline) showed the writer busy only ~47 % of the wall time with batches of ~540 exchanges closed by the 250 ms timer: the harness's 56 MiB in-flight cap (below the store's 64 MiB `queue_bytes_max`) holds fewer than `batch_max` = 1,000 exchanges at ~106 KB each, so each batch waits out `batch_interval`. If the baseline confirms it, the rate is bounded by flow control plus the batch timer, not by writer CPU, and `gen_busy_pct` does not catch it. Stop and re-plan with the maintainer before T1 (spec risk 1): the batch and queue bounds are ADR-0002 numbers and are not changed silently.
  - If the stage ranking contradicts spec §3.5 (for example SQL, not bytes, dominates), stop and re-plan T1/T2 with the maintainer before any code (spec risk 1).
- [ ] **Commit** `perf(store): per-stage ingest timers and SHA-256 probe` (the tooling of step 1, verified by a short smoke; its numbers are labelled smoke, not a baseline). Body: the feature, the knobs, the probes. The baseline (median/min, MB/s, stage split) and the T2 decision with its arithmetic go in the step 3 commit.

### Task 1: remove waste on the writer thread (opus; one commit per item)
Each item ends with the gate invocation (3 × 10 s, `s13`) and one `--features stage-timing` run; the median, min, `mb_per_s` and the moved stage go into the commit body as `measure: median=… min=… mb/s=… <stage> … → …`, and into a running ledger in the scratchpad for T3.

#### T1.0 (only if T0 step 2 < 1,000 MB/s): hardware SHA-256 on aarch64
**Files:** root `Cargo.toml`. Note: `sha2 0.11.1` has no feature for this; its `src/sha256.rs` already picks `aarch64_sha2` at run time through `cpufeatures` unless `--cfg sha2_backend="soft"` is set. So a slow probe means the detection failed or a `soft` cfg leaked in (check `RUSTFLAGS`, `.cargo/config.toml`); find out which before touching anything. If the only switch is a `--cfg`/`RUSTFLAGS` setting, stop and ask: a build-environment dependency is not acceptable for the release binary.
- [ ] Probe ≥ 1,000 MB/s; `blob_hash_is_fips_sha256` and `relay_sha256_matches_kat` (both from the dependency refresh), the RFC 4231 HMAC vectors in `preview_token.rs`, `compat.rs` pass; `cargo deny check` clean, `cargo tree -d` unchanged.
- [ ] **Commit** `build(deps): use the ARMv8 SHA-256 instructions on aarch64`.

#### T1.1 Hash each pack body once
**Files:** `src/packs.rs`: `append(tx, raw)` keeps its signature and becomes `let hash = sha256(raw); self.append_hashed(tx, raw, hash)`; new `pub(crate) fn append_hashed(&mut self, tx, raw: &[u8], hash: [u8; 32])` with `debug_assert_eq!(hash, Sha256::digest(raw))`. `src/bodies.rs`: `put` passes the hash it already computed (`bodies.rs:86`) to `append_hashed` (called at `bodies.rs:108`; the second hash is `packs.rs:128`). No change to `read_record`.
**Tests:** new `tests/hash_compat.rs`: corpus = inline 1 B, exactly 16 KiB, 16 KiB + 1, a deduplicated pair, truncated, `capture_dropped`, `relay`, 8 MiB noise, 1 MiB zeros. For each stored body: record header hash == `blobs.hash` == `bodies.blob_hash` == JSON `blob.hash` == independent `Sha256` of the submitted bytes; three of them also equal literal hex constants computed with `shasum -a 256` outside Rust. Every committed record re-reads through `read_record` (decompress + verify). Existing `packs.rs` tests (`hash_mismatch_detected_on_read`, `flipped_payload_byte_is_corrupt`, `decompression_bomb_refused`, `frame_without_content_size_refused`), `blob_hash_is_fips_sha256`, `compat.rs` unchanged and passing.
- [ ] `cargo test --locked -p crosswire-store --test hash_compat --test packs --test compat --test writer`, clippy, then measure.
- [ ] **Commit** `perf(store): hash each pack body once`.

#### T1.2 Stop base64-encoding bodies into a discarded JSON value
**Files:**
- `crates/crosswire-model/src/message.rs`: `with_body(mut self, body: Body) -> Message` already exists as `pub(crate)` (`message.rs:754`, used by redaction); make it `pub` and keep its doc comment accurate. `crates/crosswire-model/src/exchange.rs`: new `pub fn set_request(&mut self, request: Message)` (mirrors `set_response`, `exchange.rs:209`). Both documented; the model is `publish = false`. (The spec's "cheap clone whose bodies are already `Body::Blob`" needs these: outside `crosswire-model`, neither `Message` nor `Exchange` can replace a body today.)
- `src/writer.rs` `insert_exchange`: compute each side's `(hash, locator)` first (`BodyStore::put`, plus `Sha256` for inline as today), then build `stored = ex.clone()` with each `Body::Inline` replaced by the same `Body::Blob { hash, len, truncated_at, capture_dropped, relay }`, `set_state(state)`, then `serde_json::to_value(&stored)` → `serde_json::to_string(&Value)` exactly as today. No `set_field` on body or state paths. `Bytes` clones are refcount bumps; heads, diffs and interim are cloned (small).
- `src/writer.rs` `#[cfg(test)] mod json_reference` with a verbatim copy of 0.1.1's builder (`to_value(ex)` → `set_field("/state")` → `set_field(body)`), used only by the test below.
**Tests:** writer unit test `stored_json_is_byte_identical_to_0_1_1`: the T1.1 corpus plus `Normalized` and `Http1Raw` heads, `error` state, tags (one repeated), notes, interim, diffs, a request with no body and an exchange with no response; new string == reference string, byte for byte. `compat.rs` (fixture `stored_json_sha256`), `tests/writer.rs` (`error_state_is_kept_and_others_become_committed`, `http1_heads_fill_search_columns`), `crosswire-model` tests, the API OpenAPI snapshot test (no schema change).
- [ ] Tests, clippy (`-p crosswire-model -p crosswire-store`), measure.
- [ ] **Commit** `perf(store): stop base64-encoding bodies into discarded JSON`. Body notes the two model additions and why `set_field` is no longer used for bodies.

#### T1.3 `prepare_cached` for every per-exchange statement
**Files:** `src/writer.rs` (existence check, `INSERT INTO exchanges`, `INSERT OR IGNORE INTO exchange_tags`, the per-batch `meta` updates), `src/bodies.rs` (`put`: both `INSERT INTO bodies`, the dedup `SELECT`, the `UPDATE … refcount`, `INSERT INTO blobs`), `src/packs.rs` (`append`'s `INSERT OR IGNORE INTO packs`). Every call becomes `tx.prepare_cached(SQL)?.execute(..)`/`.query_row(..)` with the **same literal SQL**. Set `Connection::set_prepared_statement_cache_capacity` (rusqlite 0.41) on the writer connection to at least the number of distinct statements + 4 if the default (16) is smaller; say which in the commit body. Read paths (`BodyStore::get`, `read.rs`) are out of scope.
**Tests:** `cargo test --locked -p crosswire-store --all-features` minus the perf tests (`--test writer --test packs --test compat --test durability --test hostile --test indexes --test tags_index --test path_filter --test hash_compat --lib`). The `EXPLAIN QUERY PLAN` tests in `indexes.rs` pass unchanged (same SQL text).
- [ ] Tests, clippy, measure.
- [ ] **Commit** `perf(store): cache the writer's per-exchange statements`.

#### T1.4 (optional, only if the T1.3 stage split shows `pack_write` or zstd context setup ≥ 5 % of writer time) No `rec` copy; one `zstd::bulk::Compressor`
**Files:** `src/packs.rs`: write the 40-byte header and the frame with two `write_all` calls (or `write_vectored` with a loop to completion); keep the torn-write rollback (`set_len(self.offset)`) covering both writes; the `#[cfg(test)] fail_write_after` hook applies across the header+frame byte stream. Keep one `Compressor` (level 3) in `PackWriter`. Frames must stay identical (content size declared): assert in `hash_compat.rs` that a 1 MiB body's frame bytes equal `zstd::bulk::compress(raw, 3)`.
- [ ] `--test packs --test hash_compat --test durability`, measure.
- [ ] **Commit** `perf(store): append pack records without an extra copy`.

#### T1.5 (as built; not in the original plan) Close a batch at a byte threshold
*Why it was added:* the T0 baseline confirmed the step 4 warning: `writer_busy_pct` ≈ 50 %, batches of ~530–600 exchanges closed by the 250 ms timer because the in-flight byte budget holds fewer than `batch_max` exchanges at ~106 KB each. After T1.2 and T1.1 the writer was busy only 36–42 % of the wall time, so further CPU cuts could not move the rate. The controller added a byte trigger that only ever makes a batch **smaller** (so the ADR-0002 loss window of 250 ms / 1,000 exchanges, `queue_bytes_max` and every fsync are unchanged).
**Files:** `src/store.rs`: new `StoreConfig::batch_bytes_max` (default 32 MiB, half of `queue_bytes_max`). `src/writer.rs` `collect`: sums each queued exchange's reserved `size` (the same count `queue_bytes_max` uses) and closes the batch once it reaches `batch_bytes_max`, in addition to `batch_max` and the timer. The commit order (pack fsync → `committed_len` → `COMMIT` → events), `finish`, the journal-drain byte budget and the spill path are untouched.
**Tests:** `tests/writer.rs` `byte_heavy_batch_closes_before_the_timer` (10 s interval, 1 MiB threshold, 3 × 512 KiB: the first `Committed` holds exactly the first two ids, the third waits for the timer, close commits it). `limits_rss_store` (release) unchanged.
**Measured** (stage-timing, 3 × 10 s, same box; thermal noise between sweeps is ±15 %): 4 MiB 2307 (min 1674), 8 MiB 2474, 16 MiB 2429, 24 MiB 3228 (min 2570), 32 MiB 2642 and 3104 (min 3068), 48 MiB 3108 (writer busy falls to 88 %: the batch approaches the in-flight cap and the gap reappears). 32 MiB = half the queue budget lets the producer fill one half while the writer commits the other; it had the best or tied-best median in both sweeps and the best min.
- [x] **Commit** `perf(store): close a batch early at a byte threshold`.

**T1 exit (decides T2 for good):** with the gate invocation after the last T1 commit:
- median ≥ 2,300/s **and** every run ≥ 2,000/s **and** `gen_busy_pct ≤ 80` → **T2 is skipped.** ADR-0007 records that the pool was not needed and why; `prepare_threads` is not added.
- otherwise → T2.
- If T0 said "not needed" but T1 misses this bar, T2 proceeds; if T0 said "needed", T2 proceeds even if T1 happens to pass (the T0 rule already priced in headroom); the commit body of T1's last item states which branch was taken.

### Task 2: bounded prepare pool with ordered commit (opus; only if T1 exit says so)
**Files:**
- `src/store.rs`: `StoreConfig::prepare_threads: usize` (default `min(4, available_parallelism().map_or(1, n).saturating_sub(1))`, D6) and `prepare_inflight_bytes: usize` (default 32 MiB of raw input dispatched and not yet consumed; one item larger than the budget is still admitted alone). `TestHooks` gains `panic_in_prepare: Mutex<Option<Ulid>>` and `panic_in_prepare_twice: AtomicBool` (the inline re-prepare panics too) and `kill_before_commit` (abort after frames are written, before `COMMIT`).
- Create `src/prepare.rs`: `pub(crate) fn prepare(ex: &Exchange) -> Result<Prepared, StoreError>`, **pure** (no file, no connection, no clock, no event): `Prepared { json_parts, bodies: [Option<PreparedBody>; 2] }`, `PreparedBody::Inline { hash, len }` | `Pack { hash, raw_len, frame: Vec<u8> }`. `hash` and `frame` come from the one immutable `Bytes`. Over-64 MiB → `StoreError::Io(InvalidInput)` as today. `Pool { jobs: SyncSender<Job>, results: Receiver<(u64 gen, usize idx, Outcome)>, workers: Vec<JoinHandle<()>> }`; `Outcome = Ok(Prepared) | Err(StoreError) | Panicked(String)`; each worker loop runs `prepare` inside `catch_unwind(AssertUnwindSafe(..))` and keeps serving. `Drop for Pool` closes `jobs` and joins every worker.
- `src/writer.rs`:
  - `Writer` owns `Option<Pool>` (`None` when `prepare_threads == 0`; then `prepare` runs inline: the T1 control flow, unchanged).
  - `try_commit` bumps a `gen` counter, dispatches items in index order while dispatched-and-unconsumed raw bytes ≤ `prepare_inflight_bytes`, and **consumes strictly by index**: result *i* is taken from a reorder buffer keyed by `(gen, idx)`; results with another `gen` are dropped. For result *i*: existence check (a skipped id drops its frame), then `BodyStore::put_prepared` (dedup `SELECT`; hit → refcount bump, frame dropped; miss → `append_frame(tx, hash, raw_len, &frame)`), then the row insert with the prepared JSON. `Panicked` → one `Error` event and `prepare(ex)` inline on the writer (a second panic unwinds the writer: `DeathWatch`). `Err(e)` → `CommitError::Item(i, e)` via `per_exchange` as today.
  - **Before `try_commit` returns `Err`, it drains every result still outstanding for the current `gen`** (bounded: at most the dispatched count), so no frame or stale result survives into the retry or the next batch.
  - Markers, `Stop`, backoff, `held`, the journal and every event never pass through the pool. `drain_journal_once` → `commit` gets the pool for free.
  - The pool is dropped (workers joined) at the end of `run`, after `finish`, before the project is returned.
- `src/bodies.rs` / `src/packs.rs`: `put_prepared` and `append_frame` (the T1.1 `append_hashed` minus compression). `put`/`append` stay for tests and the serial path.
- `src/bin/killtest-child.rs`: optional 3rd arg `<prepare_threads>` and 4th `<body_len>` (defaults: library default, 20 KiB).
- `tests/perf_ingest.rs`: wire `CROSSWIRE_INGEST_PREPARE_THREADS` into `StoreConfig`; SUMMARY prints the effective value.
- `docs/format.md`: one descriptive sentence in "Write path" (step 3 there: "For each exchange not already stored: its bodies (inline or appended to a pack), then its `exchanges` row") that hashing and compression may run on helper threads before step 3 (no contract change).

**Tests (the spec §8 risk-2 matrix; every one must pass at `prepare_threads` 0 and 4):**
- Test matrix: a `matrix!` macro in each of `tests/writer.rs`, `tests/durability.rs`, `tests/hash_compat.rs` and `src/store.rs mod tests` generates `<name>::serial` (0) and `<name>::pool4` (4) from each existing test body; the helpers `cfg()`/`open()` take the thread count. **No existing assertion is edited**; the commit body lists every test now run twice.
- Stored JSON byte-identical to 0.1.1: `stored_json_is_byte_identical_to_0_1_1` at 0 and 4, plus `hash_compat.rs` at 0 and 4.
- Retry after set-aside: `worker_fault_is_set_aside_alone` — 6 exchanges, item 3 has a 64 MiB + 1 body (fault raised on a worker): only item 3 in `journal/bad/`, one gap marker, items 1,2,4,5,6 committed with `Committed` ids in submission order; then a second batch commits normally (no stale result). Plus the existing `failing_exchange_is_set_aside_alone` (`fail_item`) at 0 and 4.
- Worker panic: `worker_panic_is_reprepared_on_the_writer` — `panic_in_prepare = Some(id2)` in a 5-item batch: all 5 committed in order, exactly one `Error` event, writer healthy, `submit` still accepts. `worker_and_writer_panic_kills_the_writer` — `panic_in_prepare_twice`: the existing `dead_writer_is_reported_and_refuses` assertions (one stop, `!is_healthy()`, `submit` → `Closed`, `StoreError::Writer`).
- Shutdown with work in flight: `close_commits_in_flight_bodies` — `test_pause` holds the writer, submit 3 × 8 MiB, release and `close()` immediately: all 3 readable after reopen, no journal files, workers joined (a `Weak` on the pool's shared state is dead after `close` returns). `drop_with_in_flight_bodies` — same with `drop(store)`: each of the 3 is committed or in `journal/`, and reopen replays the latter (`journal_is_replayed_on_open_idempotently` style check). `kill_before_commit_is_truncated` (`kill_before_commit` hook): frames written, no `COMMIT`; reopen → sweep truncates past `committed_len`, no record past it is trusted, `blobs` has no row for them.
- Journal replay: `journal_is_replayed_on_open_idempotently`, `unreadable_journal_file_is_kept_and_replayed_once_readable`, `journal_drain_respects_the_byte_budget` at 0 and 4, plus `replayed_pack_bodies_use_the_pool` (plant 20 journal files with 1 MiB bodies; open with 4 threads; all committed in ULID order, files removed only after the WAL fsync).
- Durability: `kill_minus_nine` at 0 and 4, the pool4 variant with 128 KiB bodies (pack path).
- Ordering: `committed_order_matches_submission` — 5,000 mixed-size exchanges, 4 workers, `Committed` ids concatenate to submission order. `batch_boundaries`, `committed_follows_received_and_is_readable` unchanged.
- Memory: `limits_rss_store.rs` unchanged at the default thread count; a new case with 8 MiB bodies asserting the extra peak stays under `prepare_inflight_bytes + 64 MiB`.

- [ ] Step 1: `prepare.rs` + `put_prepared`/`append_frame` + inline path only (`prepare_threads` forced 0). All T1 tests pass. **Commit** `refactor(store): split pure body preparation from the writer`.
- [ ] Step 2: the pool, ordered consumption, drain-on-error, panic handling, lifetime; the matrix macro; the new tests. `cargo test --locked -p crosswire-store --all-features --lib --test writer --test durability --test hash_compat --test packs --test compat --test limits_rss_store` (debug and `--release` for `limits_rss_store`). **Commit** `perf(store): prepare bodies on a bounded pool, commit in order`.
- [ ] Step 3: measure: the gate invocation at the default (4 on the M2), then `CROSSWIRE_INGEST_PREPARE_THREADS=0`, then `CROSSWIRE_INGEST_MIX=typical`, then `--features stage-timing`. Record all in the ledger. **No commit unless the harness changed.**
- [ ] **T2 exit:** median ≥ 2,000 and min ≥ 1,800 with `gen_busy_pct ≤ 80`. If not: **stop.** Report the stage split to the maintainer with the options from spec §4.3 (zstd probe / lower level for incompressible bodies) and risk 5 (disk ceiling). Do not lower a target, a floor or an fsync to pass.

### Task 3: docs (sonnet; may be drafted in a parallel worktree, no cargo)
**Files:**
- Create `docs/adr/0007-ingest-throughput.md` (ADR format of 0006): Context (ADR-0006's miss: 698/708/722/s, ~65 MB/s; spec §3), the T0 stage split and SHA probe, what changed (T1 items with each one's measured delta; T2 taken or skipped and why, with the T0 rule and the T1 exit numbers), the gate (D2), why BLAKE3/lower zstd/raising batch bounds were rejected (spec §4.4–4.6), Consequences (extra in-flight memory bound if T2; dedup-hit CPU, D7; the Linux VM row is report-only until a baseline, D4).
- `docs/adr/0006-performance-gates-0-1-0.md`: status line + a short "Update (0.1.2): ingest asserting" pointing to ADR-0007. No rewrite of history.
- The spec's D8/D9 text fixes were made in the first commit; T3 only re-reads the spec for drift against what landed.
- `bench/README.md`: ledger table row "Ingest" → ASSERTED on the M2 (median of 3 ≥ 2,000, each ≥ 1,800, 10 s), `mb_per_s` and `typical` report-only; "Store ingest gate" section: new invocation, env knobs, invalid-run rule, the reference table (3 runs, median, min, MB/s, `typical`, `prepare_threads=0` comparison if T2 landed, T0 baseline kept as "before").
- `docs/security-gates.md`: the ingest row → measured numbers and **ASSERTED (M2)**; the paragraph after the table updated.
- `docs/superpowers/specs/2026-10-22-crosswire-design.md` §13: "1 % > 1 MiB" → "1 % 1–8 MiB (uniform)"; add "sustained body MB/s is reported; a lighter typical mix is report-only". The 2k/s figure is unchanged.
- `justfile`: `perf` passes `CROSSWIRE_PERF_ASSERT=1 CROSSWIRE_INGEST_RUNS=3 CROSSWIRE_INGEST_WINDOW_SECS=10` and `--test-threads=1` on the ingest line; comment updated.
- `CHANGELOG.md` `[Unreleased]`: `### Changed` (store writer throughput, numbers), `### Added` (`StoreConfig::prepare_threads` if T2).
*As built:* the ADR is `docs/adr/0007-ingest-batching-and-hashing.md` (named for what landed, since T2 was not built). `CHANGELOG.md` is not edited by hand (release-plz generates it from the commits). The `justfile` got a comment fix only; passing the gate env vars on the `perf` ingest line moves to T4 with the assertion. `perf_ingest.rs` got comment-only fixes (stale ~700/s prose); its "REPORT-ONLY for now" header stays until T4 step 1. `docs/format.md` is unchanged (no prepare pool, so no "Write path" sentence).
- [x] ADR-0007 created; ADR-0006 status line and "Update (0.1.2): ingest target met".
- [x] Spec drift re-read: an "Outcome" note under the ingest spec's status line.
- [x] `bench/README.md` ledger row and "Store ingest gate" section (before/after table, per-step table, reproduction command, cool-box caveat).
- [x] `docs/security-gates.md` ingest row and the paragraph after the table.
- [x] Product spec §13: "1 % 1–8 MiB (uniform)", MB/s and the typical mix reported only.
- [x] **Commit** `docs: ingest throughput results and ADR-0007`.

### Task 4: gate, review and PR (opus)
- [ ] Step 1: `tests/perf_ingest.rs` flips to asserting under `CROSSWIRE_PERF_ASSERT=1` in release: median ≥ 2,000, every run ≥ 1,800, `gen_busy_pct ≤ 80`, mix=`s13` only; the gross floor (always on in release) rises from 300/s to half the T2/T1 final median rounded down to a hundred; `sha256_backend_throughput` asserts ≥ 1,000 MB/s on `aarch64` under the same env var (guards a silent backend regression). Module docs updated (no longer "REPORT-ONLY"). **Commit** `test(store): assert the §13 ingest rate on the reference machine`.
- [ ] Step 2: run each gate **sequentially**, each to its own scratchpad log, reading exit code and summary (never `| tail`):
  - `cargo fmt --all --check`
  - `just lint` (clippy `-D warnings`, all targets, all features)
  - `cargo deny check`
  - `just doc`
  - `cargo +1.89 check --workspace --locked --all-features` (CI `msrv` job)
  - `just features`, and `cargo check --locked -p crosswire-store` (feature off) and `--features stage-timing`
  - **the asserting perf gate, run 3 separate times** (the Global Constraints invocation; each must pass; all 9 run rates go in the PR body), plus `just perf`'s ingest line once to prove the justfile wiring
  - `cargo test --locked -p crosswire-store --test compat` (0.1.0 fixture opens, every hash and `stored_json_sha256` matches, append works)
  - `cargo test --locked --release -p crosswire-store --test limits_rss_store`
- [ ] Step 3: start the workspace test **once**, in the background: `cargo test --workspace --locked --all-features --no-fail-fast > <scratchpad>/p2-workspace-test.log 2>&1`. ~40 minutes; before calling it hung, check `pgrep -P <cargo pid>`. Then `grep -E '^test result|FAILED|panicked' <log>`; zero failures required.
- [ ] Step 4: manual compatibility check (spec §6): open a project written by this branch with the 0.1.1 release binary (`crosswire serve --project <dir>`), list history, open three pack bodies. Note the result in the PR body.
- [ ] Step 5: whole-branch review (opus) against Review Focus 1–5 and ADR-0002, reading `git diff origin/main...` in full, not just the tests. Fix findings in new commits.
- [ ] Step 6: push and open the PR. Body: baseline vs final (median/min/MB/s, both mixes, `prepare_threads` 0 vs default if T2), the T2 decision and its numbers, the stage split before/after, the SHA probe, the test-matrix list, the manual 0.1.1 check, and `git log --format=%B origin/main.. | grep -ci -e claude -e co-authored` = 0.

## Outcome
Measured on the M2 (AC power, Low Power Mode off, cool machine, `s13`, 3 × 10 s, release):
- **Before** (`0f144c0`, 0.1.1 writer on `sha2` 0.11): 818 / 1016 / 924, median 924, min 818, 88–108 MB/s; `writer_busy_pct` ~50; stage-timing median 1048; `typical` median 2459 (report-only).
- **After** (T1 head): 3023 / 3482 / 3295, **median 3295, min 3023**, 314–361 MB/s, `gen_busy_pct` 11–12. Gate (median ≥ 2,000, every run ≥ 1,800) **passed**; T1 exit (≥ 2,300 / ≥ 2,000) passed, so **T2 is skipped**.
- Per step (stage-timing medians): T1.2 no discarded JSON 1148; T1.1 hash once 1382; T1.5 byte-threshold batch close (32 MiB, from a 4/8/16/24/32/48 MiB sweep) 3104.
- Root cause: at ~106 KB per exchange the 64 MiB queue holds ~530–600 exchanges, under the 1,000 batch trigger, so batches closed on the 250 ms timer and the writer idled ~50 %. JSON encode was ~30 % and double hashing ~22 % of writer time.
- Not done: T1.0 (probe 2,101 MB/s, hardware SHA already active), T1.3 (`prepare_cached`, no median gain, not committed), T1.4 (not needed), T2 (not needed); BLAKE3 ruled out (SHA-256 pinned in the format).
- Thermal: a warm machine read ~2,400/s at the same commit; take the gate on a cool box.
- On-disk format unchanged: `hash_compat.rs` literal digests, the 0.1.0 fixture, `json_reference` byte-identical.
- Recorded in ADR-0007, the ADR-0006 update, `bench/README.md` and `docs/security-gates.md`. T4 still flips the rate assertion on and raises the gross floor.
