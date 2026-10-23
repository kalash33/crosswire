# crosswire-store

The durable exchange store: a project directory holding a SQLite database,
content packs for large bodies, a spill journal and an audit log. Part of
[Crosswire](https://github.com/kalash33/crosswire/blob/main/README.md), an open-source Rust web security testing platform.

A `Store` takes finished `Exchange`s from a proxy through an `ExchangeSink`,
commits them in batches on one writer thread, announces each commit as an
`Event`, and serves them back through a keyset-paginated read API. It is built so
that a `kill -9` never corrupts a project and every loss is marked.

The on-disk format is in
[docs/format.md](https://github.com/kalash33/crosswire/blob/main/docs/format.md); the reasoning is in
[ADR-0002](https://github.com/kalash33/crosswire/blob/main/docs/adr/0002-store-durability.md).

## The durability promise

> Never corrupts a project. Anything whose `committed` event you saw survives a
> process crash; a crash loses at most one commit batch (up to 250 ms or 1,000
> exchanges) when the writer keeps up and at most the bounded queue (64 MiB or
> 10k) otherwise; power loss loses at most one checkpoint interval (30 s or
> 64 MiB). Every loss is marked by a gap row.

This is spec section 9.3, and `tests/durability.rs` checks it by killing a
writer process with SIGKILL under load.

## What it does and does not do

- Creates, opens and locks a project (`flock` on `project.lock`), refuses a
  newer format, and opens the database in SQLite defensive mode with
  `trusted_schema=OFF`.
- Runs one writer OS thread: bounded queue, journal spill, batched
  `BEGIN IMMEDIATE` transactions, pack fsync before `COMMIT`, WAL checkpoints.
- Stores bodies up to 16 KiB inline and larger ones in append-only zstd packs,
  deduplicated by SHA-256 and verified on read.
- On open: validates pack tails, truncates a torn one, replays the journal, and
  writes a gap marker if the previous run did not close.
- Broadcasts ordered events and keeps an fsynced `audit.jsonl`.
- Reads: `list` (newest first, filter by method, host, status, namespace, state),
  `get`, `get_body`, `markers`, `counts`.
- Does **not** do search beyond those filters (no full-text index), retention or
  purge, compaction, encryption at rest, or migration beyond schema version 1.
- Unix only: the lock and the file modes use unix APIs, and the crate does not
  compile elsewhere yet.
- Does not guarantee power-loss durability for every commit. See the limits in
  ADR-0002.

## Key types

| type | what it is |
| --- | --- |
| `Project` | the project directory: `create`, `open`, `open_or_create`, the lock, the writer connection, `reader()` |
| `Store` | `open(Project, StoreConfig)`, `sink()`, `subscribe()`, `list`, `get`, `get_body`, `markers`, `counts`, `audit`, `is_healthy`, `open_report`, `close` |
| `StoreConfig` | batch, queue, journal, checkpoint, event and audit limits (below) |
| `OpenReport` | what open did: sweep result, journal files drained, the unclean-restart gap marker, events sent before anyone could subscribe |
| `EventStream` | `recv`, `blocking_recv`, `try_recv`; strictly increasing `seq`, with a synthesised `Event::Lagged` when a subscriber falls behind |
| `ExchangeFilter`, `Page`, `ExchangeRow`, `Counts` | the read API's inputs and outputs; `PAGE_LIMIT_MAX` is 1000 |
| `BodyStore`, `BodySide`, `BodyLocator` | store and fetch one body, inline or in a pack |
| `PackWriter`, `BlobRef` | the append-only pack file writer and a pointer to one record |
| `sweep`, `SweepReport` | the startup validation of packs |
| `StoreError` | `Io`, `Sqlite`, `Locked`, `NewerFormat`, `AlreadyExists`, `NotAProject`, `Corrupt`, `QueueFull`, `Closed`, `Json`, `Writer`; `#[non_exhaustive]` |

## Limits and defaults

| limit | default | constant or field |
| --- | --- | --- |
| Batch interval (first exchange to commit) | 250 ms | `StoreConfig::batch_interval` |
| Batch size | 1,000 exchanges | `StoreConfig::batch_max` |
| Queue | 10,000 exchanges | `StoreConfig::queue_max` |
| Queue bytes | 64 MiB; an exchange costs its head and inline body bytes plus 1 KiB | `StoreConfig::queue_bytes_max` |
| Journal cap | 1 GiB; past it, exchanges are dropped with a gap marker | `StoreConfig::journal_cap` |
| WAL checkpoint | every 30 s or at 64 MiB of WAL, then the WAL is fsynced | `checkpoint_interval`, `checkpoint_bytes` |
| Event buffer per subscriber | 10,000 events, then `Lagged` | `StoreConfig::event_capacity` |
| Audit log rotation | 64 MiB, one `audit.jsonl.1` generation | `StoreConfig::audit_rotate_bytes` |
| Inline body | at most 16 KiB (`<=`) | `INLINE_MAX` (`bodies.rs`) |
| Pack rotation | 256 MiB | `PACK_LIMIT` (`packs.rs`) |
| Pack compression | zstd level 3 | `LEVEL` |
| Largest body stored, and largest decompressed on read | 64 MiB (`MAX_RAW`). A larger body fails its exchange alone: it is set aside in `journal/bad/` with a gap marker | `packs.rs` |
| Decompression ratio on read | none for packs: the store compressed them itself and the 64 MiB declared-size cap bounds memory (the 100:1 limit applies to the Content-Encoding decode view, not built yet) | `packs.rs` |
| Journal file read back | at most 256 MiB | `MAX_FILE` (`journal.rs`) |
| Gap window slack | 1 s before the first drop | `GAP_SLACK_MS` (`sink.rs`) |
| Database failure backoff | 250 ms doubling to 30 s | `BACKOFF_MIN`, `BACKOFF_MAX` (`writer.rs`) |
| Read connections kept | 4 | `READER_POOL_MAX` (`read.rs`) |
| Page size | clamped to 1..=1000 | `PAGE_LIMIT_MAX` |

## Security notes

- A project file is untrusted input. Pack paths are derived from the pack id and
  never read from the database; the database is opened defensive with
  `trusted_schema=OFF`; journal, pack, audit and lock files are opened with
  `O_NOFOLLOW`; a symlinked `journal/`, `packs/`, `project.db`, `-wal`, `-shm`
  or `audit.jsonl` is refused, and a non-regular journal file is never opened;
  decompression is bounded.
- Directories are created 0700 and files 0600 (unix). The audit log is 0600.
- Bodies are stored as received. Nothing is redacted on the way in; redaction is
  applied by readers and exporters (`crosswire-model`).
- `req_host` is the request's `Host` header, lowercased, so it is the message's
  claim, not proof of where the connection went. When there is no `Host` it falls
  back to the connection's target address, plus `:port` unless the port is the
  scheme default (443 with TLS, 80 plain).

## Example

```rust,ignore
use std::path::Path;
use crosswire_model::Event;
use crosswire_store::{ExchangeFilter, Project, Store, StoreConfig};

let project = Project::open_or_create(Path::new("/home/me/projects/acme"))?;
let store = Store::open(project, StoreConfig::default())?;
let sink = store.sink(); // Arc<dyn ExchangeSink>
let mut events = store.subscribe();

// `submit` can block on file I/O (a spill fsyncs), so call it from a thread
// that may block: spawn_blocking in an async runtime.
let id = exchange.id();
tokio::task::spawn_blocking(move || sink.submit(exchange)).await??;

// Accepted is not durable. Wait for the commit that carries the id.
while let Some(event) = events.recv().await {
    if matches!(&event, Event::Committed { ids, .. } if ids.contains(&id)) {
        break;
    }
}

let newest = store.list(&ExchangeFilter::default(), None, 100)?; // Page<ExchangeRow>
store.close()?; // the only call that marks the project cleanly closed
```

## Status

Pre-alpha, `publish = false`; the API is not frozen. The project format is
version 1 and is documented in `docs/format.md`. The crate depends on the
unpublished `crosswire-model` (and through it `crosswire-core` and `crosswire-h1`),
so it cannot be packaged yet.

Design: [spec section 9](https://github.com/kalash33/crosswire/blob/main/docs/superpowers/specs/2026-10-22-crosswire-design.md)
(storage, write path and durability, audit log).
