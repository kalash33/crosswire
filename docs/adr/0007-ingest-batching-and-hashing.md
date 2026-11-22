# ADR-0007: Ingest batching and hashing (0.1.2)

Status: Accepted. Closes the ingest half of
[ADR-0006](0006-performance-gates-0-1-0.md).

## Context

Spec section 13 sets an ingest target of at least 2,000 exchanges/s committed
through the real writer path at the section 13 body mix (60 % under 4 KiB,
30 % 4 to 64 KiB, 9 % 64 KiB to 1 MiB, 1 % 1 to 8 MiB uniform, all
incompressible). ADR-0006 recorded the miss at 0.1.0: about 700 exchanges/s
(698, 708 and 722), about 65 MB/s of body bytes, on the reference machine
(MacBook Air M2, 8 GiB, release build), and shipped the gate REPORT-ONLY. It
put the cause on the single writer thread being CPU-bound on SHA-256 and zstd,
and proposed parallel hashing and compression.

That 700/s was measured on `sha2` 0.10, which ran SHA-256 in software on
aarch64. The dependency refresh moved to `sha2` 0.11, which picks the ARMv8
SHA instructions at run time. Before changing the writer, the 0.1.2 work
(spec `docs/superpowers/specs/2026-11-19-crosswire-ingest-throughput-design.md`,
plan `docs/superpowers/plans/2026-11-19-crosswire-ingest-throughput.md`) added
per-stage writer timers (the non-default `stage-timing` feature), an in-process
SHA-256 probe and a 3 x 10 s run mode to `perf_ingest`, and took a baseline:

- **Baseline** (0.1.1 writer on `sha2` 0.11, commit `0f144c0`; M2, AC power,
  Low Power Mode off, cool machine, `s13` mix, 3 x 10 s): 818, 1016 and 924
  exchanges/s, median 924, 88 to 108 MB/s. The `typical` mix (report-only)
  had a median of 2459.
- **SHA-256 probe:** about 2,100 MB/s, so the hardware backend was already
  active.
- **Stage split:** the writer thread was busy only about 50 % of the wall
  time. Of its busy time, building the row JSON was about 30 % (it
  base64-encoded every inline body into a `serde_json::Value` and then threw
  those strings away) and hashing pack bodies twice was about 22 %.

The idle half was the real limit. At about 106 KB per `s13` exchange, the
64 MiB queue (and the harness's 56 MiB in-flight cap below it) holds only
about 530 to 600 exchanges, fewer than the 1,000-exchange batch trigger. So
every batch closed on the 250 ms timer while the producer waited on a full
queue, and the writer sat idle between batches. Making the writer faster
alone could not raise the rate; ADR-0006's "CPU-bound" diagnosis was half
right.

## Decision

Three changes on the single writer thread, each its own commit so each can be
reverted alone, and no worker pool:

1. **No discarded JSON** (`7299da3`). The bodies are stored first, then a clone
   of the exchange (`Bytes` clones are refcount bumps) has each inline body
   swapped for its `Body::Blob` reference and goes through the same
   `to_value` then `to_string` pipeline. The stored text is byte-identical to
   0.1.1, proved against a verbatim copy of the 0.1.1 builder.
2. **Hash each pack body once** (`562d30a`). `BodyStore::put` hands the
   SHA-256 it computed for the dedup lookup to the pack writer instead of
   hashing the same bytes again. Debug builds re-hash and assert.
3. **Close a batch at a byte threshold** (`e7d4221`). New
   `StoreConfig::batch_bytes_max`, default 32 MiB (half of `queue_bytes_max`):
   a batch commits when it reaches 250 ms, 1,000 exchanges or 32 MiB of queued
   bytes, whichever comes first. The producer fills one half of the queue
   budget while the writer commits the other. The threshold was chosen from a
   sweep of 4, 8, 16, 24, 32 and 48 MiB: small thresholds pay more fsyncs
   (36 % of writer time at 4 MiB), and at 48 MiB the batch nears the in-flight
   cap and the idle gap returns. 32 MiB had the best or tied-best median in two
   sweeps and the best minimum.

The gate is the one the spec set (plan decision D2): median of 3 runs at least
2,000 exchanges/s and every run at least 1,800, 10 s windows, `s13` mix, on
the M2. `mb_per_s`, the `typical` mix and the 4-vCPU Linux VM are
report-only. The section 13 target is unchanged; the only spec edit is the
clarification that the 1 % bucket is "1 to 8 MiB uniform", which is what the
harness has measured since 0.1.0.

Result at the head of the change (same conditions as the baseline): 3023, 3482
and 3295 exchanges/s, **median 3295, minimum 3023**, 314 to 361 MB/s, generator
busy 11 to 12 % (so the harness is not the limit). Per step, from the
stage-timing build (medians): baseline 1048, no discarded JSON 1148, hash once
1382, byte-threshold batch 3104.

## Consequences

- The section 13 ingest target is met with about 65 % headroom on the median.
  The gate invocation and the reference table are in
  [bench/README.md](../../bench/README.md).
- **No on-disk format change.** SHA-256 everywhere (pack record header,
  `blobs.hash`, `bodies.blob_hash`, the JSON `blob.hash`), zstd level 3,
  `user_version` 6. `tests/hash_compat.rs` checks each body's hash in all four
  places against an independent digest and literal `shasum` values; the 0.1.0
  fixture opens and every stored hash and `stored_json_sha256` matches.
- **No change to durability.** The byte threshold only ever makes a batch
  smaller, so the ADR-0002 loss window (at most one batch, 250 ms or 1,000
  exchanges) still bounds what a crash can lose. Every commit still fsyncs the
  pack, records `committed_len` and commits before `Committed` is sent;
  `queue_bytes_max`, the timer, the count bound and the journal spill path are
  unchanged.
- Batches of large bodies are now smaller, so there are more fsyncs per
  gigabyte. At the default threshold that cost is far below the idle time it
  removes; a slower disk would move the best threshold, and the sweep above
  is the method to re-tune it.
- The fanless M2 throttles: a warm machine measured about 2,400/s at the same
  commit. Still over the target, but the gate has to be taken on a cool
  machine, on AC power, with Low Power Mode off, or the numbers are not
  comparable.
- The writer stays single-threaded. Every file write, fsync, SQL statement
  and event runs on one thread in the 0.1.1 order, so the hardest code in the
  store did not gain concurrency.

## Alternatives considered

- **A bounded pool of worker threads for SHA-256, zstd and row JSON** (spec
  option 4.2, plan T2). Not built. The plan's exit rule after the
  single-thread changes was median at least 2,300 and every run at least 2,000;
  the result (median 3295, min 3023) cleared it with margin, and the writer
  was already busy 94 to 97 % of the time at about 3,100/s, close to the disk
  ceiling estimated in the spec (about 3,300/s at 106 KB). A pool would add
  concurrency, in-flight memory and wasted compression on dedup hits to the
  most safety-critical thread for little or no measured gain. It stays the
  next step if a slower reference machine (the Linux VM) misses.
- **BLAKE3 instead of SHA-256.** Faster per core, but SHA-256 is pinned in the
  format: the pack record header, `blobs.hash` (the dedup key),
  `bodies.blob_hash`, the stored JSON `blob.hash` and the 0.1.0 fixture. A
  project written with BLAKE3 would read as corrupt in 0.1.0 and 0.1.1. That
  is a format change with a migration, not a 0.1.x performance fix, and with
  hardware SHA-256 at about 2,100 MB/s hashing is no longer the bottleneck.
- **A hardware SHA-256 flag** (plan T1.0: `sha2-asm`, a feature or a `--cfg`).
  Dropped: the probe read about 2,100 MB/s, against a 1,000 MB/s bar for "the
  hardware backend is active". `sha2` 0.11 already selects the ARMv8 SHA
  instructions at run time, and a build-environment switch would not have
  been acceptable for the release binary anyway.
- **`prepare_cached` for every per-exchange SQL statement** (plan T1.3).
  Tried and not committed: it did not move the median beyond run-to-run
  noise. Statement preparation was not where the time went.
- **Raising the batch count or timer bounds.** Rejected: they define the
  ADR-0002 loss window. The byte threshold gets the same effect (the writer
  never waits on a timer while the queue is full) by making batches smaller,
  never larger.
- **Lowering the zstd level or re-specifying the target as MB/s.** Not
  needed. The level is unchanged, and MB/s stays a reported co-metric next to
  the unchanged exchanges/s target.
