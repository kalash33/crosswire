# Crosswire project format, version 1

A project is a directory. Everything Crosswire records lives in it. This document is the on-disk contract; `crosswire-store` is its implementation. The pack record was implemented in Task 6 and the write path, journal and audit log in Task 7.

## Directory layout

```
<project>/            mode 0700 (unix)
  project.db          SQLite database, WAL mode, mode 0600
  project.db-wal      SQLite WAL (created by SQLite)
  project.db-shm      SQLite shared memory (created by SQLite)
  project.lock        lock and status file, mode 0600
  packs/              content packs, {id:06}.pack
  journal/            one fsynced JSON file per spilled exchange not yet in the database
  journal/bad/        journal files and exchanges the writer could not commit
  audit.jsonl         append-only audit log, mode 0600
  audit.jsonl.1       the previous audit generation (after rotation)
```

Directories are created 0700 and files 0600 on unix. Other platforms use the platform default.

## Database

- `PRAGMA application_id = 0x43575052` (`CWPR`). A database with another value is not a project.
- `PRAGMA user_version = 1`. `0` means an uninitialised database (not a project). A value above 1 is refused as a newer format.
- `PRAGMA journal_mode = WAL`, `synchronous = NORMAL`.
- The project is opened with `SQLITE_DBCONFIG_DEFENSIVE` on and `trusted_schema = OFF`, so a hostile project file cannot run SQL functions from its schema or write to shadow tables.
- One writer connection (the store's writer thread) and any number of read-only connections (`query_only = ON`, `busy_timeout = 5000`, `cache_size = -16384` (16 MiB), `mmap_size = 268435456` (256 MiB)).
- Migrations are in `crates/crosswire-store/migrations/` and run in one transaction that also sets `user_version`.

### Tables (`0001_init.sql`)

`exchanges` (`WITHOUT ROWID`), one row per exchange:

| column | type | notes |
|---|---|---|
| `ulid` | TEXT PRIMARY KEY | exchange id, 26-character ULID |
| `namespace` | TEXT NOT NULL | `history`, `repeater`, ... |
| `state` | TEXT NOT NULL | exchange state |
| `scope_state` | TEXT NOT NULL | in / out / empty |
| `created_at` | INTEGER NOT NULL | unix milliseconds |
| `json` | TEXT NOT NULL | the `Exchange` JSON with `state` `committed`; each inline body is replaced by a `blob` body (`hash` = SHA-256, `len`), its bytes being in `bodies` |
| `req_method`, `req_host`, `req_path` | TEXT | denormalised for search. `req_host` is the request's `Host` header value, ASCII-lowercased with any port kept (the message identity; HTTP/2 will use `:authority`); with duplicate `Host` headers the first wins; only when there is no `Host` does it fall back to the connection's target address, lowercased, with `:port` appended unless the port is the scheme default (443 with TLS, 80 without), as a browser would have sent it |
| `resp_status` | INTEGER | NULL when no response |
| `resp_len` | INTEGER | response body length |
| `tags` | TEXT NOT NULL DEFAULT '' | |
| `notes` | TEXT NOT NULL DEFAULT '' | |

`bodies` (`WITHOUT ROWID`), one row per body side:

| column | type | notes |
|---|---|---|
| `ulid` | TEXT NOT NULL | exchange id |
| `side` | TEXT NOT NULL | `CHECK (side IN ('req','resp'))` |
| `inline` | BLOB | small body stored in the row |
| `blob_hash` | BLOB | SHA-256 of a body stored in a pack |
| `len` | INTEGER NOT NULL | body length |

Primary key `(ulid, side)`. At most one of `inline` and `blob_hash` is set (`CHECK`).

`blobs` (`WITHOUT ROWID`), the index of content in packs: `hash BLOB PRIMARY KEY`, `pack_id INTEGER NOT NULL`, `offset INTEGER NOT NULL`, `len INTEGER NOT NULL`, `refcount INTEGER NOT NULL DEFAULT 1`.

`packs`: `id INTEGER PRIMARY KEY`, `committed_len INTEGER NOT NULL DEFAULT 0` (bytes of the pack file that the database has committed and trusts; the tail beyond it is validated on open, kept while its records are intact, and truncated at the first bad record). There is no path column: the pack file is always `packs/{id:06}.pack` (for example `packs/000001.pack`), derived from the id. A project file is untrusted input, so readers must never open a path stored in the database.

`markers` (`WITHOUT ROWID`), gap and error rows: `ulid TEXT PRIMARY KEY`, `kind TEXT NOT NULL`, `from_ulid TEXT`, `to_ulid TEXT`, `detail TEXT NOT NULL DEFAULT ''`. A `gap` is a time window: exchanges that were submitted and not recorded have ids (creation times) in `from_ulid..=to_ulid` (`from_ulid` NULL means from the start). An exchange created before `from_ulid` but submitted after it can also be missing.

`meta` (`WITHOUT ROWID`): `key TEXT PRIMARY KEY`, `value TEXT NOT NULL`. Keys:

- `last_committed_ulid`: the greatest (in ULID order) exchange id whose row and bodies are committed (empty string initially). Updated in the transaction that commits the batch; never moves backwards. For readers; gap markers do not use it.
- `last_commit_watermark`: written by every commit: the oldest exchange id still held only in memory at that commit, or the commit time minus one second if that is earlier (absent until the first commit). An exchange lost in a crash after that commit has an id at or after it, unless it was created more than a second before it was submitted.

### Indexes

- `exchanges_created_at` on `exchanges (created_at)`
- `exchanges_req_host` on `exchanges (req_host)`
- `exchanges_resp_status` on `exchanges (resp_status)`
- `exchanges_namespace_created_at` on `exchanges (namespace, created_at)`

## Lock file

`project.lock` is held with an exclusive, non-blocking `flock` for the lifetime of the open `Project`. The lock is unix-only in v1 (the crate does not compile elsewhere); Windows support is a later task. A second open fails with `Locked`. The file holds three lines:

```
<pid>
<start unix seconds>
clean=0|1
```

Opening writes `clean=0` (and `Store::open` writes it again); only `Store::close` writes `clean=1`. If the previous contents say `clean=0`, the previous run died, so `Project::open` runs `PRAGMA integrity_check` and `Store::open`, before replaying the journal, writes a `gap` marker from `last_commit_watermark` to the restart (detail `unclean shutdown`).

## Pack record (implemented in Task 6)

Content is stored in `packs/{id:06}.pack`, append-only. Each record:

```
magic       4 bytes   "CWPK"
clen        4 bytes   u32 little-endian, length of the zstd frame
sha256     32 bytes   SHA-256 of the raw (uncompressed) bytes
frame    clen bytes   one zstd frame
```

The `blobs` row points at the record start with `offset` and `len` (the whole record). Reading verifies the SHA-256. A pack rotates to a new file at 256 MiB. Only bytes up to `packs.committed_len` are trusted.

## Write path (Task 7)

One writer thread owns the writer connection. Exchanges reach it through a bounded queue (10,000 exchanges and 64 MiB, an exchange costing its message bytes plus 1 KiB). It commits a batch when 1,000 exchanges are waiting or 250 ms after the first one arrived. Per batch, in order:

1. `BEGIN IMMEDIATE`.
2. A pending `gap` marker (exchanges were dropped): from the earliest dropped id (or one second before the first drop, if earlier) to now.
3. For each exchange not already stored: its bodies (inline or appended to a pack), then its `exchanges` row.
4. `last_committed_ulid` advances to the greatest id in the batch; `last_commit_watermark` is rewritten.
5. The current pack is fsynced; `packs.committed_len` is raised to the end of the last record written to each pack.
6. `COMMIT`. If the batch came from journal files, the WAL is fsynced and only then are the files deleted. Then the `committed` event is sent.

A failure caused by one exchange (an oversized body, a JSON or constraint error) moves that exchange to `journal/bad/`, records a gap, and commits the rest. Any other failure (disk full, I/O error) is treated as the database being unavailable: queued exchanges are written back to `journal/` (held in memory if that fails too), journal files stay, one `error` event is sent per failure streak, and the writer retries with a backoff of 250 ms doubling to 30 s.

`Store::close` drains the queue and the journal. Dropping a `Store` without `close` drains only the in-memory queue; the journal waits for the next open.

`PRAGMA wal_checkpoint(PASSIVE)` runs every 30 s or when the WAL passes 64 MiB, followed by an fsync of the WAL; `close` ends with `wal_checkpoint(TRUNCATE)`.

## Journal (Task 7)

When the queue has no room, the sink writes the exchange to `journal/<ulid>.json.tmp` (one JSON `Exchange`, bodies inline, mode 0600), fsyncs it, renames it to `journal/<ulid>.json` and fsyncs the directory before returning. The journal is capped at 1 GiB; past that the exchange is dropped, one `error` event is sent per episode, and the writer's next commit writes a `gap` marker.

The writer commits journal files oldest ULID first whenever the queue is below a quarter full, at most 1,000 files and 64 MiB of files (at least one) per batch, and all of them on every open. Replay is idempotent: an id already in `exchanges` is skipped and its file deleted. Leftover `.tmp` files were never acknowledged and are deleted on open. A file that cannot be read or committed is moved to `journal/bad/` with an `error` event.

## Audit log (Task 7)

`audit.jsonl` holds one JSON object per line: `ts` (unix milliseconds), `ulid`, `actor`, `action`, `target`, `result`. Each row is `fdatasync`ed before the call returns. When a row would take the file past 64 MiB it is renamed to `audit.jsonl.1` (replacing any previous one) and a new file is started.
