# ADR-0002: store durability

Status: Accepted

## Context

The recorder sits in the path of a security tester's traffic. Two things must
hold at once: the proxy must never stop relaying because the disk is slow or
full, and a project must never be corrupt or silently incomplete, even after
`kill -9`. A project is also the unit people share and archive, so a crash cannot
be allowed to leave it in a state that needs hand repair.

Spec section 9.3 states the promise in one sentence. This ADR records how
`crosswire-store` keeps it, what it chose not to do, and where the promise is
weaker than it reads.

> Never corrupts a project. Anything whose `committed` event you saw survives a
> process crash; a crash loses at most one commit batch (up to 250 ms or 1,000
> exchanges) when the writer keeps up and at most the bounded queue (64 MiB or
> 10k) otherwise; power loss loses at most one checkpoint interval (30 s or
> 64 MiB). Every loss is marked by a gap row.

## Decision

The sentence is kept as written. The write path below is what makes it true.

1. **One writer OS thread.** `crosswire-store-writer` owns the only write
   connection and the pack writer. It receives exchanges, batches them, and does
   all database and pack I/O for them. The relay never performs file I/O for a
   queued exchange.
2. **A bounded queue, reserved before `try_send`.** `submit` first reserves one
   slot and the exchange's byte cost (message bytes plus 1 KiB) against 10,000
   exchanges and 64 MiB with compare-and-swap, and only then does a `try_send` on
   a channel sized to the slot limit. Because a reservation always precedes the
   send, the channel cannot overflow and the byte budget is exact. The
   reservation is returned when the exchange is committed or written to the
   journal. The ids reserved are tracked, so the oldest exchange held only in
   memory is known at any moment.
3. **Spill to a fsynced journal when the queue is full.** The exchange is written
   to `journal/<ulid>.json.tmp`, fsynced, renamed, and the directory fsynced,
   all before `submit` returns. `Received` is sent only after that. The writer
   commits journal files oldest first whenever the queue is under a quarter full,
   and all of them on every open. Past 1 GiB of journal the exchange is dropped,
   `submit` returns `Full`, one `Error` event is sent, and the writer records a
   gap marker. The proxy keeps relaying and stops recording; it is never blocked.
4. **Batches under `BEGIN IMMEDIATE`.** A batch is committed 250 ms after its
   first exchange or at 1,000 exchanges. In order: `BEGIN IMMEDIATE`; a pending
   gap marker; each exchange's bodies and row (an id already stored is skipped,
   which makes replay idempotent); `last_committed_ulid` and
   `last_commit_watermark`; fsync of the pack; `packs.committed_len` raised to
   the end of the last record written; `COMMIT`; then, if the batch came from
   journal files, an fsync of the WAL and only then deletion of those files; then
   the `Committed` event. Nothing is announced before `COMMIT` returned, and
   `COMMIT` never runs before the pack bytes it references are on disk.
5. **`committed_len` is the trust boundary of a pack.** Bytes before it were
   fsynced before the transaction that recorded it. The sweep never reads them;
   it validates only what lies beyond, and truncates at the first bad record.
6. **WAL with `synchronous=NORMAL`.** `COMMIT` does not fsync the WAL. The writer
   runs `wal_checkpoint(PASSIVE)` every 30 s or when the WAL reaches 64 MiB and
   then fsyncs the WAL itself, so readers holding the checkpoint back cannot
   stretch the interval. After a process crash the OS still has every committed
   write, so `committed` events are safe. After a power loss, commits since the
   last WAL sync can be lost: at most one checkpoint interval, which is the
   window in the sentence. SQLite guarantees a WAL database is not corrupted by
   this, only rolled back to an earlier commit.
7. **Gap markers are time windows from the commit watermark.** Exchange ids are
   ULIDs minted when the exchange is created. Every commit writes
   `last_commit_watermark`: the oldest id still only in memory at that moment, or
   the commit time minus one second, whichever is earlier. After a crash, an
   exchange that was accepted and lost has an id at or after the watermark of the
   last commit that survived. On the next open after an unclean run, `Store::open`
   writes a `Gap` marker from that watermark to the restart, before replaying the
   journal. A drop at runtime (journal full, journal unwritable, an exchange that
   cannot be stored) records a window from the earliest lost id (for a drop at the
   queue or journal, the earlier of that id and one second before the drop) to the
   time the marker is written.
8. **Audit log.** `audit.jsonl` is append-only and each row is `fdatasync`ed
   before the call returns. It rotates once, at 64 MiB, to `audit.jsonl.1`.
9. **Startup sweep.** On open, before the writer starts: pack tails are validated
   from `committed_len`, a torn tail is truncated, exchanges that pointed into it
   are set to `error` with a gap marker, orphan blob rows are deleted, leftover
   `.tmp` journal files (never acknowledged) are deleted, journal files are
   replayed, and `PRAGMA integrity_check` runs only if `project.lock` says the
   previous run did not close. Only `Store::close` records a clean shutdown;
   dropping a `Store` commits what is in memory, leaves the journal for the next
   open and leaves the project marked unclean, so the next open writes a gap.

A failure caused by one exchange (a body over 64 MiB, a JSON or constraint error)
sets that exchange aside in `journal/bad/`, records a gap, and commits the rest. A
database failure (disk full, I/O error) writes queued exchanges back to the
journal, keeps journal files where they are, sends one `Error` per failure streak,
and retries with a backoff from 250 ms doubling to 30 s.

The promise is tested by `crates/crosswire-store/tests/durability.rs`
(`kill_minus_nine`): a child process records 5,000 exchanges with 20 KiB bodies
and is killed with SIGKILL after 1,000 `committed` ids were reported. The test
reopens the project and asserts `integrity_check` is `ok`, every id seen
committed is present with byte-exact bodies, and every accepted id that is
missing falls inside the gap marker's window. The write-path behaviours (spill,
cap, replay, failure handling, event ordering) are in `tests/writer.rs` and the
unit tests in `src/store.rs`.

## Alternatives considered

**Journal-first durability for every exchange.** Write each exchange to the
journal with an fsync before `submit` returns, and commit from the journal. This
would make every accepted exchange survive any crash, and the gap window would
shrink to the exchanges being relayed at that moment. Rejected. It puts a disk
flush (two with the rename and directory sync) on the relay's path for every
exchange, where the chosen design pays one pack fsync per batch of up to 1,000.
Matching the ingest target in the spec would need group commit, which is a second
durable write path to build, test and fuzz next to the first, and every exchange
would then be written twice. The part-time budget is the deciding reason: the
spill journal already exists for the case where the queue is full, and a stated,
bounded loss window with an honest gap marker is a smaller thing to get right
than a second durability mechanism. No throughput figure for the alternative was
measured; the argument rests on cost structure, not a benchmark.

**`synchronous=FULL`.** This would fsync the WAL on every commit and shrink the
power-loss window from one checkpoint interval to one batch. Rejected for now. It
adds a flush to the writer's critical path on every batch, which is where a slow
disk makes the queue fill and spills start, and the promise the product needs is
about process crashes, not power loss: a recorded session is mostly reproducible
by sending the requests again. The cost of the rejected choice is the 30 s window
above. The switch is one pragma, and spilled exchanges already get the FULL
behaviour (their files are kept until the WAL is fsynced), so the decision can be
reversed without changing the format.

**Separate pack-writer and spill threads.** Spec section 9.3 describes them. The
implementation uses the single writer thread for bodies and does the spill on the
submitting thread, because one thread that owns the connection and the pack
removes a class of ordering bugs (a row that references a pack offset another
thread has not synced yet). The price is the second limitation below.

## Consequences

- A `committed` event is safe against process death, but it is not a promise
  about power loss. Only exchanges committed from the journal are fsynced before
  their event.
- Memory is bounded: 64 MiB of queued exchanges and 10,000 events per subscriber.
  Disk is bounded: 1 GiB of journal. When both are full, recording stops with a
  gap marker and the proxy keeps relaying.
- Every loss is a row a reader can find (`markers`, `Event::Marker`), and the gap
  window is by ULID time, which is what the kill -9 test asserts.
- The durability promise is tested for process death only. The power-loss bound
  follows from SQLite's WAL semantics and the order of fsyncs above; no test pulls
  the power, and none could be run in CI.
- Ingest throughput of the single writer thread against the spec's 2,000
  exchanges per second target has not been measured in this plan.

Known limitations:

- **macOS flush strength.** The writer connection sets
  `checkpoint_fullfsync=ON`, so SQLite's checkpoint syncs issue `F_FULLFSYNC`
  on macOS (plain `fsync` there does not force the drive to flush its cache).
  `fullfsync` (every WAL commit sync) is not set; with `synchronous=NORMAL`
  SQLite does not sync the WAL on commit anyway. The explicit syncs in this
  crate go through `File::sync_all` and `sync_data`, which on macOS issue
  `F_FULLFSYNC` (a flush of about 1.4 ms against about 40 microseconds for
  plain `fsync`, measured on a development laptop). The WAL, packs, journal,
  audit log and the database file after a checkpoint are therefore flushed
  hard. Linux is not affected.
- **The spill fsyncs on the submitting thread.** `submit` can block for a file
  create, two fsyncs, a rename and a directory fsync when the queue is full.
  Callers must run it where blocking is acceptable (`spawn_blocking`); the
  `record_exchange` recipe in `crates/crosswire-recorder-tests/src/lib.rs` does.
- **The keyset list can miss late exchanges.** `list` pages newest first by ULID,
  which is creation time. An exchange committed late (a journal replay after a
  crash, or a long-running exchange) has an id older than rows a reader has
  already seen, so a poller that only asks for ids newer than the last one it saw
  does not find it. A consumer that must see every exchange tails `Committed`
  events instead and uses `list` for display.
- **Gap windows are by creation time.** An exchange that was created before the
  window started and submitted after it can be lost outside the window. The
  one-second slack covers ordinary requests, not a long download.
- **Failure to write back.** If a batch cannot be committed and cannot be written
  back to the journal either (the disk is gone), the exchanges stay in memory and
  are lost if the process then dies. A gap marker is attempted at shutdown and, if
  it cannot be written, `close` returns an error and leaves the project unclean.
- **Unreadable journal files.** A journal file that cannot be parsed is moved
  to `journal/bad/` and recorded as a gap from the ULID in its name, so the loss
  is findable but the exchange is not replayed. A file that fails with a
  transient I/O error (`EMFILE`, `EIO`, `EACCES`) stays in `journal/` and is
  retried with backoff; after 10 retries (about two minutes) it is treated as
  unparseable, so a file that stays unreadable longer than that is set aside
  with a gap rather than replayed later. While it is retried the writer's
  backoff also delays queue commits by up to the current delay.
- **Spec 9.3 refinement.** The spec says the gap is written by the startup sweep
  between the last committed ULID and the restart. The implementation writes it
  in `Store::open` from `last_commit_watermark`, which is never later than the
  last committed ULID and is the safer bound when commits and ULID order differ.
