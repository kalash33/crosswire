# Crosswire project format, version 1

A project is a directory. Everything Crosswire records lives in it. This document is the on-disk contract; `crosswire-store` is its implementation. Sections on the pack record and the journal describe formats implemented in later tasks of the recorder plan and are fixed here so the format is reviewed once.

## Directory layout

```
<project>/            mode 0700 (unix)
  project.db          SQLite database, WAL mode, mode 0600
  project.db-wal      SQLite WAL (created by SQLite)
  project.db-shm      SQLite shared memory (created by SQLite)
  project.lock        lock and status file, mode 0600
  packs/              content packs, {id:06}.pack
  journal/            one fsynced JSON file per exchange not yet in the database
  audit.jsonl         append-only audit log, mode 0600
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
| `json` | TEXT NOT NULL | the `Exchange` JSON without inline bodies |
| `req_method`, `req_host`, `req_path` | TEXT | denormalised for search |
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

`markers` (`WITHOUT ROWID`), gap and error rows: `ulid TEXT PRIMARY KEY`, `kind TEXT NOT NULL`, `from_ulid TEXT`, `to_ulid TEXT`, `detail TEXT NOT NULL DEFAULT ''`.

`meta` (`WITHOUT ROWID`): `key TEXT PRIMARY KEY`, `value TEXT NOT NULL`. Keys:

- `last_committed_ulid`: id of the last exchange whose row and bodies are committed (empty string initially). Journal files with a greater id are replayed on open.

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

Opening writes `clean=0`; a clean close writes `clean=1`. If the previous contents say `clean=0`, the previous run died, so open runs `PRAGMA integrity_check` and replays the journal.

## Pack record (implemented in Task 6)

Content is stored in `packs/{id:06}.pack`, append-only. Each record:

```
magic       4 bytes   "CWPK"
clen        4 bytes   u32 little-endian, length of the zstd frame
sha256     32 bytes   SHA-256 of the raw (uncompressed) bytes
frame    clen bytes   one zstd frame
```

The `blobs` row points at the record start with `offset` and `len` (the whole record). Reading verifies the SHA-256. A pack rotates to a new file at 256 MiB. Only bytes up to `packs.committed_len` are trusted.

## Journal (implemented in Task 7)

Before the sink returns for an exchange it writes `journal/<ulid>.json` (one JSON `Exchange`, bodies inline) and fsyncs the file and the directory. The writer thread moves exchanges into the database in batches, advances `meta.last_committed_ulid`, and deletes the journal files it has committed. On open after an unclean run, journal files with an id greater than `last_committed_ulid` are replayed.
