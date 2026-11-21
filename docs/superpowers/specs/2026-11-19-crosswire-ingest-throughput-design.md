# P2: store ingest throughput (Crosswire 0.1.x)

**Status:** accepted 2026-11-19 with the controller decisions D1–D10 applied (drafted 2026-11-18; the implementation plan is `docs/superpowers/plans/2026-11-19-crosswire-ingest-throughput.md`, which lists the decisions). Nothing in here has been implemented or measured yet. Every number is either quoted from the repo (and cited) or marked as an estimate. **Why:** the §13 ingest row ("ingest ≥ 2k exchanges/s at a body mix of 60 % < 4 KiB, 30 % 4–64 KiB, 9 % 64 KiB–1 MiB, 1 % > 1 MiB", product spec `2026-10-22-crosswire-design.md` §13) is the last §13 performance row that is still REPORT-ONLY. ADR-0006 recorded the miss (about 700 exchanges/s, three runs at 698, 708 and 722, about 65 MB/s of body bytes, on the M2 reference machine) and committed 0.1.1 to "re-specify ingest as sustained MB/s with a lighter, more typical mix, and/or raise the writer's throughput with parallel hashing and compression". 0.1.1 shipped with the UI half closed and this half open (ADR-0006, "Update (0.1.1)").

**Base:** a branch `feat/ingest-throughput` from `main` at `e4e1e6d`, after the 0.1.2 dependency refresh (PR #35) merged. That refresh moved the workspace to `sha2 0.11.1` and `rusqlite 0.41.1` (bundled SQLite 3.54.1) and added the SHA-256 known-answer tests `blob_hash_is_fips_sha256` (`crosswire-store/tests/packs.rs`) and `relay_sha256_matches_kat` (`crosswire-proxy/src/capture.rs`). The draft was written against `2b4e2eb` (the 0.1.1 release merge); every file and line below was re-checked on `e4e1e6d` (the store's writer, bodies and packs code did not move). §7 Q1 is settled by that merge (D1).

## 1. Goals
1. The §13 ingest target, **≥ 2,000 committed exchanges/s at the §13 mix**, is met on the M2 reference machine through the real writer path, and `perf_ingest` asserts it under `CROSSWIRE_PERF_ASSERT=1` (`just perf`). The target is not lowered.
2. The ingest SUMMARY also reports sustained **MB/s of body bytes** and a second, lighter **"typical" mix** row, so a maintainer can see throughput in both units. These are report-only.
3. No on-disk format change. A 0.1.0 project still opens, every stored hash is the same SHA-256 of the same bytes, a project written by this build opens in 0.1.0 and 0.1.1, and the stored `exchanges.json` text is byte-identical to what 0.1.1 would have written.
4. No change to durability. The order "pack fsync → `committed_len` → `COMMIT` → `Committed` event" is kept, the journal-delete-after-WAL-fsync rule is kept, and so are the batch bounds (250 ms / 1,000) and the queue bounds (10,000 / 64 MiB), because the ADR-0002 promise is stated in those numbers.

## 2. Non-goals (later, or never in 0.1.x)
BLAKE3 or any other hash (§4.5). A new pack record version, a new `user_version`, or a migration. Changing `batch_interval`, `batch_max`, `queue_max`, `queue_bytes_max`, `checkpoint_*` or `synchronous`. Relaxing `F_FULLFSYNC` or any fsync. More than one writer connection or more than one open pack. Dictionary compression. Streaming bodies larger than memory. Changes to the proxy relay or the recorder thread (§3.4 explains why they are not the bottleneck). Any new crate in the dependency graph beyond what §7 Q1 settles.

## 3. Where the time goes today

### 3.1 What is measured (quoted, not re-run)
| # | Fact | Source |
|---|---|---|
| M1 | §13 mix through the real sink and writer: 698 / 708 / 722 exchanges/s, about 65 MB/s of bodies, M2 (Mac14,2), 8 GiB, release, on 0.1.1's `sha2 0.10.9` (software SHA-256 on aarch64) | `bench/README.md` "Store ingest gate", ADR-0006 |
| M2 | Same harness with bodies capped at 65 KiB: about 2,400 exchanges/s | ADR-0006 Root causes, `bench/README.md` |
| M3 | The disk sustains about 350 MB/s for a 300 MB write with a full fsync on the same machine | same |
| M4 | The harness bodies are unique xorshift noise ("poorly compressible", `fill()` in `perf_ingest.rs`), so nothing dedups and zstd finds no matches | `crates/crosswire-store/tests/perf_ingest.rs` |
| M5 | The harness draws the 1 % class uniformly from 1 to 8 MiB (`MIB + 1..=8 * MIB`). §13 says only "> 1 MiB" | `perf_ingest.rs` `body_class_len`, spec §13 |

No profile (Instruments, `perf`, flamegraph) is checked into the repo or quoted in any doc I could find. The per-stage split below is therefore **inferred from the code**, not profiled. T0 (§6) turns it into a measurement before any optimisation lands.

### 3.2 The expected body size at this mix (arithmetic from M5)
Uniform buckets give means of about 2 KiB, 34 KiB, 544 KiB and 4.5 MiB. Weighted 0.60 / 0.30 / 0.09 / 0.01, that is about 1.2 + 10.2 + 49 + 46 ≈ **106 KB per exchange**, which matches ADR-0006's "about 100 KB". **Over 90 % of the bytes come from the 10 % of exchanges over 64 KiB, and about 45 % from the 1 % bucket alone.** About 66 % of exchanges are at or under the 16 KiB `INLINE_MAX` and go into SQLite. The other 34 % go to packs, and they carry about 98 % of the bytes. 2,000/s × 106 KB ≈ 212 MB/s.

### 3.3 The writer's work per exchange (read from the code)
All of this runs on the one `crosswire-store-writer` thread (`writer.rs` `try_commit` → `insert_exchange` → `BodyStore::put` → `PackWriter::append`):

| Stage | Where | Cost driver |
|---|---|---|
| a. Existence check `SELECT 1 FROM exchanges WHERE ulid = ?` | `writer.rs:670` | per exchange |
| b. **`serde_json::to_value(ex)` of the whole exchange, bodies included.** `bytes_serde` writes every body as a base64 string with a hand-written per-`char` `String::push` loop (`crosswire-model/src/b64.rs` `encode`), and serde_json then copies it into a `Value::String`. That value is **thrown away** a few lines later when `set_field` swaps in the `Body::Blob` reference | `writer.rs:878`, `bytes_serde.rs`, `b64.rs` | per body byte: an encode pass, a copy of 1.33× the body, then a free |
| c. Inline bodies (≤ 16 KiB): `INSERT INTO bodies`, plus `Sha256::digest(bytes)` for the JSON `hash` | `bodies.rs:80`, `writer.rs:903` | per byte (SHA), per exchange (SQL) |
| d. Pack bodies: **`Sha256::digest(raw)` in `BodyStore::put`** for the dedup key | `bodies.rs:86` | per byte |
| e. `SELECT … FROM blobs WHERE hash = ?` (dedup) | `bodies.rs:88` | per pack body |
| f. `zstd::bulk::compress(raw, 3)`, which builds a new compression context on every call | `packs.rs:121` | per byte |
| g. **`Sha256::digest(raw)` again in `PackWriter::append`**, the same bytes hashed a second time | `packs.rs:128` | per byte |
| h. Copy the frame into a new `rec` Vec (header + frame), then `write_all` | `packs.rs:137` | per byte (a memcpy) |
| i. `INSERT OR IGNORE INTO packs`, `INSERT INTO blobs`, `INSERT INTO bodies` (a dedup hit runs `UPDATE blobs SET refcount` instead, `bodies.rs:95`) | `packs.rs:139`, `bodies.rs:109,120` | per pack body |
| j. `INSERT INTO exchanges` (6 history indexes, two of them covering), plus `exchange_tags` | `writer.rs:934,958` | per exchange |
| k. Once per batch: pack `sync_all`, which is `F_FULLFSYNC` on macOS; `committed_len`; `COMMIT` | `writer.rs:720` | per batch |

Every SQL call goes through `Transaction::execute` or `query_row` with a literal string, so **each statement is re-prepared on every call** (about 6 to 9 prepares per exchange). The statement cache (`prepare_cached`) is not used.

**`sha2` ran in software on the reference machine when M1 was measured.** 0.1.1 pinned `sha2 = "0.10"` with no features (locked at 0.10.9). In 0.10.9, `src/sha256.rs` selects the ARMv8 SHA2-instruction backend on `aarch64` only under the `asm` feature (`#[cfg(all(feature = "asm", target_arch = "aarch64"))]`). Otherwise it falls through to `soft`. On x86-64 the same version picks SHA-NI at run time without a feature, so the M2 paid a software SHA-256 that the 4-vCPU Linux reference VM probably did not (if its CPU has SHA-NI). **Since the dependency refresh, `main` locks `sha2 0.11.1`**, whose `src/sha256.rs` detects the aarch64 SHA2 extension at run time by default (`cpufeatures::new!(sha2_hwcap, "sha2")`, falling back to `soft`), unless a `--cfg sha2_backend="soft"` is set. So M1 (about 700/s) is a 0.10.9 number; T0 re-measures the baseline on the 0.1.1 writer with `sha2 0.11.1` and proves in-process that the hardware backend is active (an MB/s probe).

### 3.4 What is not the bottleneck (inferred, with reasons)
- **The disk.** M3 gives 350 MB/s with a full fsync, against the 65 MB/s achieved. Fsync is once per batch, and a batch is up to about 56 MiB under the harness's flow control.
- **The proxy → store handoff.** The relay does a `try_send` into the recorder's 1,024-slot queue (`crosswire-proxy/src/record.rs`). One recorder thread calls `StoreSink::submit`, which does a CAS reservation and a `try_send` (`sink.rs`), all O(1) per exchange. The harness measures the sink and writer only, so the proxy is not in the 700/s figure at all. The handoff becomes a problem only when the writer falls behind: then `submit` spills each exchange to a fsynced journal file on the recorder thread (fsync, rename, directory fsync), the recorder queue fills, and the relay drops records with a gap marker. That is the *consequence* of a slow writer, and making the writer faster is the cure.
- **The harness's own generator.** `fill()` is an xorshift loop on the submitting thread, far faster than 65 MB/s. It could become a limit once the writer goes several times faster, so T0 reports it (§6).

### 3.5 A two-point model (inferred from M1 and M2)
If writer time per exchange is `a + b·bytes`, M1 (1/700 s at about 106 KB) and M2 (1/2,400 s at a capped mean of about 18 KB, assuming the cap clamps draws to 65 KiB) give **b ≈ 11.5 ms per MB (about 87 MB/s marginal)** and **a ≈ 0.2 ms per exchange**. To reach 2,000/s at 106 KB, the writer needs `a + b·0.106 ≤ 0.5 ms`. With `a` unchanged, that means **b ≤ about 2.8 ms/MB (≥ about 350 MB/s on the writer thread)**: about a 4× cut in per-byte writer cost. The fixed part also matters: at `a = 0.2 ms`, the serial ceiling is 5,000/s even with free bytes.

Rough per-MB costs, estimated rather than measured (T0 measures them): software SHA-256 on an M2 core at about 2.5–4 ms per pass, **done twice** (stages d and g) → **5–8 ms**; base64 encode plus copy, about 1–2 ms (b); zstd level 3 on noise, about 1–3 ms (f); memcpy and `write`, about 0.3 ms (h). That adds up to about 7–13 ms/MB, which brackets the inferred 11.5 ms/MB. **The likely ranking is: SHA-256 (two software passes) > the discarded base64 > zstd > I/O.** If T0 disagrees, §4's recommendation is revisited before T2.

## 4. Options

### 4.1 (f, new) Remove the redundant work on the writer thread, single-threaded and format-neutral
1. **Hash each pack body once.** `PackWriter::append` takes the hash `BodyStore::put` already computed (stage g goes away).
2. **Hardware SHA-256 on aarch64.** Done by the dependency refresh: the workspace is on `sha2 0.11.1`, which selects the ARMv8 SHA2 instructions at run time. Same algorithm, same digest (the FIPS 180-4 known-answer tests prove it), so the format is untouched. T0's probe confirms the backend is active; if it is not, enabling it is a T1 item (D1), never a reason to go back to 0.10 or to add `sha2-asm`.
3. **Never base64 a body into a throwaway `Value`.** Build the row JSON from a cheap clone of the `Exchange` (`Bytes` clones are refcount bumps) whose bodies are *already* `Body::Blob { hash, len, truncated_at, capture_dropped, relay }`, with `state` set. Then serialise it through the same `to_value` → `to_string(&Value)` pipeline as today. Because the `Value` handed to `to_string` is the same, the stored text is byte-identical, including serde_json's sorted map-key order (no `preserve_order` feature in the workspace).
4. **`prepare_cached` for every per-exchange statement** (the a, c, e, i and j SQL). This attacks the fixed `a`.
5. Write the 40-byte header and the frame without building `rec` (two `write_all` calls, or `write_vectored`), and reuse one `zstd::bulk::Compressor` per thread.

*Trade-offs:* small, local, testable diffs with no concurrency. *Estimated* effect: per-byte writer cost drops from 7–13 to about 2–4 ms/MB (zstd plus hardware SHA plus I/O), so roughly **1,600–2,300/s** at the §13 mix. That is likely but not certain to clear 2,000/s on its own, and it leaves no headroom for the Linux VM's 4 vCPUs.

### 4.2 (a) Move SHA-256, zstd and row-JSON building into a bounded pool, with an ordered commit
A `prepare` step is a pure function from an `Exchange` to `Prepared { json: String, bodies: [PreparedBody; 2] }`, where `PreparedBody` is either `Inline { hash, len }` or `Pack { hash, raw_len, frame: Vec<u8> }`. It touches no file and no connection. A fixed pool of `prepare_threads` std threads (no rayon, no new crate) runs it.

- **Where it hooks in.** Inside `Writer::try_commit` the writer sends each item of the batch to the pool, in order and bounded by an in-flight budget (§4.2.2). Then it **consumes the results strictly in batch order**: for result *i* it does the existence check, the dedup `SELECT`, appends the precomputed frame (or bumps `refcount` and drops the frame on a dedup hit), and runs the inserts. The pool keeps working on items i+1… while the writer does SQL for item *i*, so preparation overlaps the serial part within a batch.
- **Ordering.** The commit order, the `Committed` id order, `last_committed_ulid`, `CommitError::Item(i, _)` indices and the `per_exchange` classification are the same as today, because the writer still walks `items` by index. Markers, `Stop`, backoff and `held` never pass through the pool.
- **Failures.** A prepare error (for example a body over `MAX_RAW`, `InvalidInput`) is returned as that item's result and becomes `CommitError::Item(i, e)`, as now. **A worker panic is caught on the worker** (`catch_unwind`), which keeps serving, and reported for that item. **The writer re-prepares that item once, inline on the writer thread**, sends one `Error` event and commits the batch as usual, so nothing is lost. **If the inline re-prepare panics too, the writer unwinds** and the existing `DeathWatch` handles it, exactly like a writer panic today (D9). On a retry after an item is set aside, the remaining items are prepared again: this is a rare path and stays simple.
- **The journal replay path** (`drain_journal_once` → `commit`) gets the same benefit for free.
- **What does not move.** Pack append, rotation, `committed_len`, fsync, every SQL statement and every event stay on the writer thread, in the order they run today.

#### 4.2.1 Sizing
The default is `prepare_threads = min(4, available_parallelism − 1)` (saturating, so a 1-CPU host gets 0; D6/D8): 4 on the 8-core M2 and 3 on a 4-vCPU VM, leaving at least one core for the writer and the proxy runtime. There is no CLI flag; only the perf harness reads an override (`CROSSWIRE_INGEST_PREPARE_THREADS`), the library never reads the environment. `0` means "prepare inline on the writer thread", which is the 4.1 behaviour. It is the fallback, and the comparison row in the harness. This is a new `StoreConfig` field. The crate is `publish = false`, and every in-repo construction uses `..StoreConfig::default()`.

#### 4.2.2 Memory bound
Today the transient peak per exchange is the base64 string (1.33× the body), the frame, and the `rec` copy of the frame. With the pool, prepared frames that are waiting for the writer are capped by **`prepare_inflight_bytes` (default 32 MiB of raw input dispatched and not yet consumed)**, plus one item even when it is larger. A frame is freed as soon as the writer appends it. The worst-case extra over 0.1.1 is therefore about 32 MiB plus one 64 MiB record, and the removed base64 copy offsets part of it. `limits_rss_store.rs` must still pass unchanged.

*Trade-offs:* this is the change that makes the per-byte cost leave the critical thread. The writer's serial cost becomes `a` (lowered by 4.1 item 4) plus a memcpy and `write` per byte. *Estimate:* serial ≈ 0.2–0.25 ms per exchange, so **about 3,000–4,000/s at the §13 mix on the M2**, at which point the disk (M3, 350 MB/s ≈ 3,300/s at 106 KB) becomes the ceiling. The cost is concurrency in the most safety-critical thread of the product, more memory in flight, and dedup hits now paying a compression on a worker before the writer discovers the hit. That compression is off the critical path, but real traffic (repeated JS bundles) dedups often, so it is real CPU.

### 4.3 (b) Lower the zstd level, or skip compression for small or incompressible bodies
The format pins "one zstd frame" with a declared content size, not a level (`docs/format.md` "Pack record"; `packs.rs` `const LEVEL: i32 = 3` is internal). Level 1, or a negative "fast" level, writes frames every zstd decoder reads, including 0.1.0's `decompress_bounded`. Bodies at or under 16 KiB are never compressed today (they are inline), so "small" is already covered. For incompressible bodies, a probe (compress the first 64 KiB at level 1; if the ratio is > 0.97, compress the whole body at a negative level) keeps text at level 3. A hand-built raw-block frame is possible but means writing zstd frame headers by hand, and is rejected as needless risk.
*Trade-offs:* real wire bodies are often already `gzip`/`br`-encoded (the store keeps raw framed wire bytes, `docs/format.md` "Bodies recorded by the proxy are the raw framed wire bytes"), so incompressible input is realistic, not just a harness artefact. But zstd at level 3 may already skip quickly on noise, and the gain is unknown until T0 measures it. With 4.2 in place, zstd runs on the pool and the level matters only for total CPU (and on 4 vCPUs). Level 1 on text costs some disk. **Conditional:** done (T4) only if T0 shows zstd is more than 25 % of prepare time on the incompressible buckets.

### 4.4 (c) Batch inserts per transaction
**Already done.** One `BEGIN IMMEDIATE … COMMIT` covers up to 1,000 exchanges or 250 ms (`writer.rs` `collect`, `try_commit`), and the fsync is per batch. Raising the batch bounds would change the ADR-0002 loss window ("a crash loses at most one commit batch (up to 250 ms or 1,000 exchanges)"), so it is out (§2). What remains of (c) is the per-statement overhead *inside* the batch: re-preparing about 6–9 statements per exchange. `prepare_cached` (4.1 item 4) removes that. Multi-row `INSERT … VALUES (…),(…)` would save a little more, but makes per-exchange fault attribution (`CommitError::Item(i)`) harder, so it is rejected.

### 4.5 (d) Re-specify the metric as sustained MB/s with a realistic mix
*For:* the writer is volume-bound (ADR-0006), so MB/s is the honest unit, and the §13 "1 % > 1 MiB" bucket is unbounded in the spec while the harness picks 1–8 MiB uniform (mean 4.5 MiB). That one choice puts about 45 % of all bytes in 1 % of exchanges. *Against:* ADR-0006 says the target "will be re-specified in 0.1.1 from measured behaviour, in the open, not tuned to the current result". A re-spec *instead of* a fix lowers the bar a user feels. **Recommendation:** do not replace the §13 target. Do (i) add `mb_per_s` as a reported co-metric (already printed), (ii) write the harness's "1 % 1–8 MiB uniform" into the §13 text as a clarification (D5) (it documents what has been measured since 0.1.0, it does not lower anything), and (iii) add a `typical` mix row, report-only, proposed as 70 % < 4 KiB, 25 % 4–64 KiB, 4.5 % 64 KiB–1 MiB, 0.5 % 1–4 MiB, half of the bodies compressible text (§7 Q3). Re-specifying becomes the fallback only if the gate in §5 misses after T3.

### 4.6 (e) BLAKE3: out for 0.1.x
BLAKE3 is several times faster than SHA-256 per core, but it **changes `blobs.hash`**, and the format freezes SHA-256 in five places:
- `docs/format.md` "Pack record": `sha256  32 bytes  SHA-256 of the raw (uncompressed) bytes`, and reads verify it (`packs.rs:340` `read_record`).
- `bodies.blob_hash` is "SHA-256 of a body stored in a pack", and `BodyStore::get` compares it with the record's hash (`bodies.rs`).
- The `exchanges.json` `blob` body carries `hash` = SHA-256 (`docs/format.md` `json` column). That JSON is the frozen message wire format (`user_version` 6, "schema_version 2").
- `blobs.hash` is the dedup primary key, so mixing algorithms would silently stop deduplicating across versions.
- The 0.1.0 fixture pins it. In `tests/fixtures/project-0.1.0/expected.json`, `blobs[0]` = `d48e24e5…f420` is exactly `resp_body_sha256` of exchange 2, and `compat.rs` `opens_0_1_0_project` checks every blob and body hash.

A project written with BLAKE3 would read back in 0.1.0/0.1.1 as `Corrupt("body hash mismatch")`, and the startup sweep would truncate the pack tail as bad records. Doing it properly needs a new record magic or version, `user_version` 7 with a migration, a dual-algorithm reader, a downgrade refusal and a new fixture: a format change, not a 0.1.x performance fix. Hardware SHA-256 (4.1 item 2) also shrinks the gap that motivates it. **Out of scope.**

## 5. Recommendation and acceptance

### 5.1 Recommendation
Do **4.1 first (T1–T2), then 4.2 (T3)**. 4.3 is conditional on T0 (T4). 4.5 is done as a co-metric and a clarification, not as a replacement. 4.6 is not done. Reasons:
- 4.1 removes about two thirds of the estimated per-byte writer work: one of the two software SHA passes, the other made about 5× cheaper in hardware (estimate), and the discarded base64. It does this with no concurrency and no format risk, and each step is independently measurable.
- 4.2 then takes the remaining per-byte work (zstd, SHA) off the single thread, which is what gives headroom on the 4-vCPU VM and against real bursts. On its own, 4.2 would parallelise wasted work, so the order matters.

**Expected result (estimate, to be confirmed by T0 and T5):** about 1,600–2,300/s after T2 and about 3,000/s or more after T3 at the §13 mix on the M2, about 300 MB/s. The disk (M3) becomes the next ceiling.

### 5.2 Numbers that must hold
| Run | Machine | Rate (§13 mix) | Status |
|---|---|---|---|
| `just perf` ingest, `CROSSWIRE_PERF_ASSERT=1` | M2 reference | **median of 3 runs ≥ 2,000 committed exchanges/s** over a 10 s window, and every run ≥ 1,800 | **ASSERTED** |
| same | 4-vCPU Linux reference VM | measured and recorded | REPORT-ONLY until a baseline exists (§7 Q4) |
| `mb_per_s` | both | reported | REPORT-ONLY |
| `CROSSWIRE_INGEST_MIX=typical` | both | reported | REPORT-ONLY |
| `CROSSWIRE_INGEST_PREPARE_THREADS=0` (serial comparison) | M2 | reported | REPORT-ONLY |
| gross-regression floor (release, always on) | any | raised from 300/s to about half the new M2 median (rounded down to a hundred), set in Tn from the measurement | ASSERTED (as today) |
| `just perf-smoke` (CI) | ubuntu | ingest not run (unchanged) | — |

**The exact invocation that proves the gate:**
```sh
CROSSWIRE_PERF_ASSERT=1 CROSSWIRE_INGEST_RUNS=3 CROSSWIRE_INGEST_WINDOW_SECS=10 \
  cargo test --locked --release -p crosswire-store --test perf_ingest -- --nocapture --test-threads=1
```
`just perf` is changed to pass `CROSSWIRE_PERF_ASSERT=1` on this line (today it runs ingest without it, because nothing asserts). Without the env var the harness keeps today's defaults (one run, 3 s window) and only asserts the mix and the floor. Disk: a 10 s run at about 212 MB/s writes about 2 GB to the temp dir, which is freed after each run.

The SUMMARY line keeps every existing field and adds `run=`, `mix=s13|typical`, `prepare_threads=`, `gen_busy_pct=` (the generator thread's busy share, so a harness-bound run is visible), and `stage_ms=` (from T0's counters). A run is **invalid**, not passing, if `gen_busy_pct > 80`.

`bench/README.md` (the ledger row and the "Store ingest gate" section), `docs/security-gates.md` (the ingest row), ADR-0006 (a new "Update (0.1.x): ingest asserting" section) and the justfile comment move ingest from REPORT-ONLY to ASSERTED on the M2, with the new numbers.

## 6. Format, compatibility and security guarantees (and the test that proves each)

| Guarantee | Proof |
|---|---|
| A 0.1.0 project opens and reads back unchanged | `compat.rs` `opens_0_1_0_project`, **unchanged and passing** (it also appends an exchange and closes, so the new writer writes into a 0.1.0 project) |
| Hashes are unchanged: every pack header hash, `blobs.hash`, `bodies.blob_hash` and JSON `blob.hash` is SHA-256 of the raw body | New `tests/hash_compat.rs`: a fixed corpus (inline, exactly 16 KiB, 16 KiB+1, deduplicated, truncated, `capture_dropped`, `relay`, 8 MiB noise, 1 MiB zeros) written with `prepare_threads` 0 and 4. Each stored hash is compared with an independent `sha2::Sha256` over the submitted bytes, **and with literal SHA-256 hex constants** for three bodies (so a backend swap that broke SHA-256 would fail too). The existing known-answer tests `blob_hash_is_fips_sha256` and `relay_sha256_matches_kat` keep passing. |
| Stored JSON is byte-identical to 0.1.1 | New writer unit test: a test-only copy of 0.1.1's `insert_exchange` JSON builder (`to_value(ex)` → `set_field`) is the reference, and for the corpus above plus `Normalized` heads, `error` state, tags and notes, the new builder's string equals it byte for byte |
| Projects written by this build open in 0.1.1 | Same test file: the same frames decode through the unchanged `read_record`/`decompress_bounded` (declared content size present, ≤ 64 MiB), the hash verifies, and every body reads back through `Store::get_body`. Manual check in the T5 release notes: open a P2-written project with the 0.1.1 binary. |
| What is hashed, and when, is unchanged: the hash is of the exact bytes that are compressed and stored | `prepare` takes one immutable `Bytes` and computes `hash` and `frame` from that same slice. A test decompresses every committed record and re-hashes it (the sweep's own `read_record`). `packs.rs` tests (`hash_mismatch_detected_on_read`, `flipped_payload_byte_is_corrupt`, `decompression_bomb_refused`, `frame_without_content_size_refused`) are unchanged. |
| Durability order: pack fsync before `COMMIT`, `Committed` only after `COMMIT`, journal files deleted only after the WAL fsync | Workers do no I/O. `durability.rs` `kill_minus_nine` and every `writer.rs` test are unchanged. A new `kill_minus_nine` variant runs with `prepare_threads = 4` and pack-sized bodies. A new unit test (`#[cfg(test)]` hook) kills a batch after the frames are written and before `COMMIT`, and checks the sweep truncates them (records past `committed_len` never become trusted). |
| Per-exchange fault isolation is kept | The existing `fail_item` hook test, plus a new one where the fault comes *from a worker* (a body over 64 MiB): only that exchange goes to `journal/bad/`, a gap is recorded, the rest commit, and the order of `Committed` ids is unchanged |
| A worker panic loses nothing; a repeated panic behaves like a writer panic (D9) | New `panic_in_prepare` hook: the item is re-prepared on the writer, the whole batch commits in order, exactly one `Error` event is sent and `submit` still accepts. With `panic_in_prepare_twice` (the inline re-prepare panics too) the writer is marked dead and `submit` returns `Closed`, the same assertions as the existing `panic_next_batch` test |
| Memory stays bounded | `limits_rss_store.rs` unchanged and passing with the default `prepare_threads`, plus a case with 8 MiB bodies that asserts the extra peak stays under `prepare_inflight_bytes` + 64 MiB |
| Ordering | `batch_boundaries` and `committed_follows_received_and_is_readable` unchanged. A new test submits 5,000 mixed-size exchanges with 4 workers and checks that the `Committed` ids arrive in submission order |

No change to `docs/format.md` is expected, except one sentence in "Write path" noting that hashing and compression can run on helper threads before step 3. That is descriptive and not part of the contract. `user_version` stays 6.

## 7. Open questions for the maintainer (all resolved; the decision is noted after each)
1. **`sha2` route.** The default is **move the workspace to `sha2 0.11`** (already in `Cargo.lock`, so `cargo deny` sees no new crate, and it uses hardware SHA on aarch64 by default). This touches `crosswire-ca`, `crosswire-tools` and the tests, and overlaps `feat/dep-refresh`: if that branch is already bumping `sha2`, P2 rebases onto it and drops T1's dependency part. The alternative is `sha2 0.10` + `features = ["asm"]`, which adds `sha2-asm` (a build-script C/asm crate) to the graph. **Resolved (D1):** P2 starts after the dependency refresh, which landed `sha2 0.11.1` (`75d02c0`, merged in `e4e1e6d`). T0 measures SHA-256 MB/s in-process; if the hardware backend is not active, enabling it is a T1 item.
2. **Gate shape.** The default is median of 3 ≥ 2,000/s, each run ≥ 1,800/s, 10 s windows. Is a min-of-3 gate preferred despite the flake risk? **Resolved (D2):** median of 3 ≥ 2,000/s and every run ≥ 1,800/s, 10 s windows, M2 only.
3. **The `typical` mix** (§4.5) is a proposal, not data. Is there any real-traffic capture to derive it from? If not, it stays report-only and is labelled as a proposal in `bench/README.md`. **Resolved (D3):** no real traffic is available; the mix stays REPORT-ONLY.
4. **The Linux reference VM.** §13 says "both reference machines must pass", but no ingest number exists for the VM. The default is: assert on the M2, record the VM, and assert there after one recorded baseline. If the VM misses at 2 workers, is 4.3 (a lower zstd level) acceptable to close it? **Resolved (D4):** the VM is report-only until a baseline exists.
5. **The §13 text clarification** ("1 % 1–8 MiB uniform, incompressible"): is this a clarification the maintainer will accept in the product spec, or does it need its own ADR line? **Resolved (D5):** clarify the §13 text to "1 % 1–8 MiB uniform" (a docs task).
6. **`prepare_threads` exposure.** It defaults to `StoreConfig` only, with no CLI flag. Should `crosswire serve` expose it (for constrained hosts), or is that later? **Resolved (D6, D8):** no CLI flag; the default is `min(4, available_parallelism − 1)`, `0` = serial, and the env override is read by the harness only.
7. **Dedup-hit CPU.** With 4.2, a body that dedups is still compressed on a worker. The default is to accept this. The alternative is a small writer-side LRU of recently stored hashes, consulted before dispatch. That needs the hash first, which is a second pass, and is rejected as complexity for now. **Resolved (D7):** accepted; wasted CPU off the writer thread is cheap and correctness is unchanged.

## 8. Risks
1. **The estimates in §3.5 are wrong in shape.** If T0 shows a different ranking (for example SQLite page writes for inline bodies dominate `a`), T2/T3 are re-planned before code. That is why T0 exists and blocks everything else.
2. **Concurrency in the writer (the riskiest part).** Ordered consumption, retry after a set-aside item, panics, shutdown (`finish` with items in flight), the journal-replay path, and the pool's lifetime against `Store::close`/`Drop` (workers must be joined before the project is returned; a dropped store must not hang on a stuck worker). Mitigation: workers are pure (no I/O, no shared mutable state, just a channel in and a channel out); `prepare_threads = 0` keeps the 0.1.1 control flow as a test matrix axis (every `writer.rs` and `durability.rs` test runs at both 0 and 4); and a full code review against ADR-0002 is required before merge.
3. **Byte-identical JSON** depends on serde_json's `Value` map ordering (sorted keys; no `preserve_order` in the graph). Both builders go through the same `Value`, so they stay equal to each other even if that changed. To also catch drift from 0.1.0's text, the reference test pins literal `stored_json_sha256` values from `expected.json` wherever the fixture inputs are deterministic.
4. **Harness-bound measurement.** At about 212 MB/s, the single generator thread, four workers, the writer and the counter share eight cores (four of them efficiency cores). `gen_busy_pct` exposes this. If the generator is the limit, it pre-generates a ring of bodies (unique per exchange through a per-exchange 8-byte salt in the first bytes, so dedup still never hits).
5. **Disk ceiling.** M3 is one sequential write. Fsync per batch with SQLite WAL traffic interleaved may land below 350 MB/s. If T3 lands between 1,800 and 2,000/s and the stage counters show fsync dominating, the remedy is a maintainer ruling (§7 Q2/Q4), not a loosening of fsync.
6. **Thermal throttling** on a fanless M2 over 3 × 10 s runs. Runs are spaced 5 s apart and the SUMMARY records each run.

## 9. Task outline (for the plan that follows)
- **T0: Harness and baseline (blocks everything).** Extend `perf_ingest.rs` with `CROSSWIRE_INGEST_RUNS`, `CROSSWIRE_INGEST_WINDOW_SECS`, `CROSSWIRE_INGEST_MIX` (`s13` | `typical`) and `CROSSWIRE_INGEST_PREPARE_THREADS` (wired once T3 exists), plus the `gen_busy_pct` and `run=` fields. Add `#[doc(hidden)]` writer stage counters (ns in JSON build, SHA, zstd, pack write, SQL, fsync, commit), reported as `stage_ms=`, with no effect on behaviour. Add a criterion micro-bench `benches/ingest_stages.rs`: software vs hardware `sha2`, zstd level 3/1/−5 on noise and on text at 64 KiB, 1 MiB and 8 MiB, `b64::encode` + `to_value` on 1 MiB, and `prepare_cached` vs `execute` for the exchange insert. The maintainer records the 0.1.1 baseline (3 runs, both mixes) and the stage split in `bench/README.md`. **Exit:** the §3.5 ranking is confirmed or corrected in this spec.
- **T1: Hash once and hardware SHA.** `PackWriter::append` takes the precomputed hash; the `sha2` bump is already on `main` (D1), so hardware SHA is a T1 item only if T0's probe shows the backend is not active. Tests: `hash_compat.rs` (literal digests), all `packs.rs` tests, `compat.rs`.
- **T2: No throwaway base64; cached statements; no `rec` copy.** The new JSON builder plus the byte-identity reference test; `prepare_cached` throughout the writer, bodies and packs paths; the two-write record append; a reusable `Compressor`. Measure (T0 invocation, `prepare_threads` n/a).
- **T3: Prepare pool with ordered commit.** `StoreConfig::{prepare_threads, prepare_inflight_bytes}`; the pure `prepare`; in-order consumption in `try_commit`; worker-panic handling (re-prepare once on the writer, D9); pool lifetime in `finish`/`Drop`; every `writer.rs` and `durability.rs` test parameterised over 0 and 4; the new ordering, worker-fault, `panic_in_prepare`, kill-before-commit and RSS tests. Measure.
- **T4 (conditional on T0): Incompressibility probe and lower level** per §4.3, only if zstd is more than 25 % of prepare time on the incompressible buckets. Tests: the frame reads back through the 0.1.1 `read_record`, and the text bodies keep level 3 (compressed size check).
- **T5: Reference measurement.** The maintainer runs the §5.2 invocation on the M2 (3 runs, both mixes, `prepare_threads` 0 and default) and on the Linux VM if available, then opens a P2-written project with the 0.1.1 binary.
- **Tn: Gate.** Flip `perf_ingest` to assert the §5.2 rows under `CROSSWIRE_PERF_ASSERT=1`; `just perf` passes it; raise the gross floor from the T5 median; update `bench/README.md`, `docs/security-gates.md`, the ADR-0006 update section, the §13 clarification (subject to §7 Q5) and the CHANGELOG under `[Unreleased]`. Run the full CI gate locally (fmt, clippy, doc, deny, the workspace tests, e2e) per the repo's finish checklist. Conventional Commits with real bodies, no attribution trailers.
