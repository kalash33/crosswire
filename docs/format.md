# Crosswire project format, version 4

A project is a directory. Everything Crosswire records lives in it. This document is the on-disk contract; `crosswire-store` is its implementation.

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
  browser-profile/    the launcher's browser profile (optional); excluded from exports
  project.db.v<n>.bak pre-migration copy of project.db (only after an upgrade from version n; see Migrations)
```

Directories are created 0700 and files 0600 on unix. Other platforms use the platform default.

`browser-profile/` is created (mode 0700) by `crosswire browser` as the
`--user-data-dir` of the browser it launches. It holds the browser's own state —
cookies, session storage and cache — not Crosswire records, so it is **excluded
from exports** and is not part of the on-disk record contract; `crosswire browser
reset-profile` deletes it.

## Database

- `PRAGMA application_id = 0x43575052` (`CWPR`). A database with another value is not a project.
- `PRAGMA user_version = 4`. `0` means an uninitialised database (not a project). A value above 4 is refused as a newer format; a value of 1, 2 or 3 is an older project, migrated up to 4 on open (see [Migrations](#migrations)).
- `PRAGMA journal_mode = WAL`, `synchronous = NORMAL`, `checkpoint_fullfsync = ON` (checkpoint syncs use `F_FULLFSYNC` on macOS; no effect elsewhere), `busy_timeout = 5000` on the writer connection. `Store::open` also sets `journal_size_limit` to 1 MiB so the WAL file does not stay at its high-water size.
- The project is opened with `SQLITE_DBCONFIG_DEFENSIVE` on and `trusted_schema = OFF`, so a hostile project file cannot run SQL functions from its schema or write to shadow tables.
- One writer connection (the store's writer thread) and any number of read-only connections (`query_only = ON`, `busy_timeout = 5000`, `cache_size = -16384` (16 MiB), `mmap_size = 268435456` (256 MiB)). There is also at most one *control* connection per [`Control`](#control-tables-0002_sidecarsql) handle: a second write connection, hardened and configured like the writer's (WAL, `synchronous = NORMAL`, `busy_timeout = 5000`), used only for the small, rare writes to the `settings`, `tokens` and `drafts` tables and for patching an exchange's `tags`/`notes`. It never writes bodies or packs, so it never contends with the writer for long; the 5 s busy timeout lets it wait out a batch commit.
- Migrations are in `crates/crosswire-store/migrations/` and run in one transaction that also sets `user_version`.

#### Migrations

A fresh project is created at the current `user_version` with every migration applied in one transaction. An existing project at an older `user_version` is migrated up on open, one step per transaction (each step also advances `user_version`, so an interrupted upgrade leaves a consistent earlier version). Before the first upgrade step, the pre-migration database is copied to `project.db.v<n>.bak` in the project directory (a plain file copy taken under the project lock, after a `wal_checkpoint(TRUNCATE)` folds the WAL into the main file and the write connection is closed), so a mistaken or interrupted upgrade can be rolled back by hand. A symlinked `.bak` path is refused rather than followed. The `.bak` is an ordinary regular file and is not part of the live project: opening a project with one present succeeds and never re-snapshots an already-current project.

- `0001_init.sql` → `user_version` 1: `exchanges`, `bodies`, `blobs`, `packs`, `markers`, `meta`.
- `0002_sidecar.sql` → `user_version` 2: `settings`, `tokens`, `drafts` (additive; see [Control tables](#control-tables-0002_sidecarsql)).
- `0003_history_indexes.sql` → `user_version` 3: the four structured-query indexes (additive; see [Indexes](#indexes)). The upgrade step also runs `ANALYZE` so the planner has statistics for the new indexes on an existing (populated) project — without them an open-ended range such as `resp_len >= ?` is served by a primary-key scan rather than the index.
- `0004_tag_index.sql` → `user_version` 4: the normalized `exchange_tags` side table (additive; see [Indexes](#indexes)). The upgrade step backfills it, in bounded chunks, from the `tags` JSON of every existing row — so a project created before version 4 answers the `tag = ?` query from the table immediately after open.

### Tables (`0001_init.sql`)

`exchanges` (`WITHOUT ROWID`), one row per exchange:

| column | type | notes |
|---|---|---|
| `ulid` | TEXT PRIMARY KEY | exchange id, 26-character ULID |
| `namespace` | TEXT NOT NULL | `history`, `repeater`, ... |
| `state` | TEXT NOT NULL | exchange state: `committed` when the writer stores it; the startup sweep sets `error` on an exchange whose pack body was lost |
| `scope_state` | TEXT NOT NULL | in / out / empty |
| `created_at` | INTEGER NOT NULL | unix milliseconds |
| `json` | TEXT NOT NULL | the `Exchange` JSON with `state` `committed`; each inline body is replaced by a `blob` body (`hash` = SHA-256, `len`), its bytes being in `bodies`; the body's `truncated_at`, `capture_dropped` and `relay` are copied to the `blob` body (see [Exchange JSON](#exchange-json)) |
| `req_method`, `req_host`, `req_path` | TEXT | denormalised for search. `req_host` is the request's `Host` header value, ASCII-lowercased with any port kept (the message identity; HTTP/2 will use `:authority`); with duplicate `Host` headers the first wins; only when there is no `Host` does it fall back to the connection's `target_addr` (the configured host, not the resolved IP), lowercased, with `:port` appended unless the port is the scheme default (443 with TLS, 80 without), as a browser would have sent it |
| `resp_status` | INTEGER | NULL when no response |
| `resp_len` | INTEGER | response body length: the body's `relay.len` when present (the capture is not the whole body), else the stored length |
| `tags` | TEXT NOT NULL DEFAULT '' | |
| `notes` | TEXT NOT NULL DEFAULT '' | |

#### Exchange JSON

The `json` column and journal files hold the `crosswire-model` `Exchange` schema. Fields added after the first schema are optional on read (absent means the default shown), so older JSON still loads:

- Bodies (`{"type":"inline",...}` and `{"type":"blob",...}`): `truncated_at` (integer or null, default null): the capture was cut off and this many bytes were kept; `capture_dropped` (bool, default false): the capture window was full, so the stored bytes are not the body and its length and hash come from `relay`; `relay` (object or null, default null): `{"len": <bytes relayed>, "sha256": "<base64 SHA-256>"}`, the length and hash of the whole body as it passed through the relay path, present when the capture is not the whole body. `inline` bodies previously had only `bytes`; `blob` bodies gain `relay`.
- Messages: `interim` (array of base64 strings, default `[]`): interim (1xx) response heads forwarded before the final response, each the raw head bytes including the empty line, in the order sent. Redaction rebuilds each with the same header policy as the head.
- Bodies recorded by the proxy are the raw framed wire bytes (chunk framing and trailers included). A body that was cut short (a side closed or timed out before it ended) has `relay` and neither `truncated_at` nor `capture_dropped`; one whose first byte was never relayed is `none`, so the cut shows only in the exchange's `error` state.
- Exchanges the proxy generates itself (a 502, 504 or 400 page) carry a response with `fidelity` `generated` and the exchange state `error`; a tunnel (SNI passthrough, an undelimitable stream, a 101 upgrade) is an exchange whose request message has the head `{"type":"tunnel"}` and no response, with `bytes_in`, `bytes_out`, `duration_ms` and `reason` (`ws`, `sni-passthrough`, `undelimitable`).
- Connection facts (`connection`): `target_addr` is the host as configured or requested (a name or an address literal, with brackets for IPv6), not the address it resolved to; `peer_addr` (string or null, default null) is the address actually connected to.
- `upstream_chain_result` `{"type":"overridden"}` gains `reason` (string, default `""`): the chain validation error that was overridden, at most 1 KiB.

#### Proxy markers

`crosswire-proxy` writes `error` and `limit-exceeded` markers through the sink. They carry no ULID window (`from_ulid` and `to_ulid` are NULL) and their `detail` is one of these, with `{host}`, `{port}` and `{peer}` filled in (substituted text is capped at 1 KiB):

| kind | `detail` | when |
| --- | --- | --- |
| `limit-exceeded` | `request from {peer} could not be parsed: {code}` | a request head or body hit a parser limit (`message head too large`, `too many headers`, `line too long`, `too many chunks`, `trailer section too large`) |
| `error` | `request from {peer} could not be parsed: {code}` | a request head or body failed to parse for any other reason |
| `limit-exceeded` | `pipelining: {n} requests queued on one connection from {peer} (max {m})` | more than `max_pipelined` requests were queued on one connection |
| `error` | `timeout: {phase} {host}:{port}` | a timeout after the response began or during the request body; `phase` is `request body`, `response body` or `total` |
| `error` | `upstream {host}:{port} closed mid-body: {error}` or `upstream {host}:{port} failed mid-body: {error}` | the upstream ended or failed in the middle of a response body |
| `error` | `undelimitable response from {host}:{port}: {code}` | a response could not be delimited, so the connection became an opaque tunnel |
| `error` | `undelimitable request to {host}:{port}: invalid Content-Length` | a request's body could not be delimited (an invalid `Content-Length`); its head was sent and the connection became an opaque tunnel |
| `error` | `extra bytes after response from {host}:{port}` | the upstream sent bytes after a complete response; the connection was dropped |
| `error` | `rewritten request head did not re-parse` | the proxy's own request rewrite produced a head the parser rejects; the original head was recorded |
| `error` | `passthrough upstream failed for {host}:{port}: {reason}` | a CONNECT was answered, then the passthrough connect to the upstream failed |
| `error` | `leaf issue failed for {host}: {reason}` | a CONNECT was answered, then no certificate could be issued for the host |
| `error` | `Browser rejected the Crosswire certificate for {host} — is the CA trusted?` | `rejections_per_window` client handshakes for one host were rejected within `rejection_window`; at most once per window per host |

`{code}` is the message of the `crosswire-h1` `ParseError` (for example `message head too large`, `line too long`, `invalid chunk size at bytes 0..2`), and `{error}` is the I/O or parse error text. The same events also set a state on the exchange they belong to; the marker is the searchable record of why.

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

`markers` (`WITHOUT ROWID`), gap and error rows: `ulid TEXT PRIMARY KEY`, `kind TEXT NOT NULL`, `from_ulid TEXT`, `to_ulid TEXT`, `detail TEXT NOT NULL DEFAULT ''`. A `gap` is a time window: exchanges that were submitted and not recorded have ids (creation times) in `from_ulid..=to_ulid` (`from_ulid` NULL means from the start). An exchange created before `from_ulid` but submitted after it can also be missing. The store itself writes only `gap` rows (detail `unclean shutdown`, `exchanges were dropped: ...`, or `pack torn`); a `pack torn` marker is a point marker whose `from_ulid` is the affected exchange and `to_ulid` is NULL. Other components submit markers of any kind (`kind` is stored as `gap`, `limit-exceeded` or `error`, the kebab-case wire names) through the sink (`ExchangeSink::submit_marker`); they are written as given, and a submitted marker whose id is already stored is skipped. A submitted marker is **memory-only until the writer commits it**: it takes a queue slot, is never spilled to the journal, and is refused (`full`) when the queue is full, so a marker raised just before a crash, or while the queue is full, is lost. Markers the proxy writes are listed under [Proxy markers](#proxy-markers).

`meta` (`WITHOUT ROWID`): `key TEXT PRIMARY KEY`, `value TEXT NOT NULL`. Keys:

- `last_committed_ulid`: the greatest (in ULID order) exchange id whose row and bodies are committed (empty string initially). Updated in the transaction that commits the batch; never moves backwards. For readers; gap markers do not use it.
- `last_commit_watermark`: written by every commit: the oldest exchange id still held only in memory at that commit, or the commit time minus one second if that is earlier (absent until the first commit). An exchange lost in a crash after that commit has an id at or after it, unless it was created more than a second before it was submitted.

### Control tables (`0002_sidecar.sql`)

These three tables hold the sidecar backend's control state. They are written only by the control connection (never the writer thread) and carry no body or pack data.

`settings` (`WITHOUT ROWID`), UI and server key/value settings:

| column | type | notes |
|---|---|---|
| `key` | TEXT PRIMARY KEY | setting name |
| `value` | TEXT NOT NULL | setting value (opaque to the store) |
| `updated_at` | INTEGER NOT NULL | last write, unix milliseconds |

The keys `crosswire serve` and the API use today:

| key | value |
|---|---|
| `scope` | the scope document as JSON (`include` and `exclude` rule lists, see the [`crosswire-scope` README](../crates/crosswire-scope/README.md)), written by `PUT /api/v1/scope` and loaded when `serve` opens the project; an unparseable value is ignored with a warning and the scope starts empty |
| `enforce_scope_strictly` | `true` or `false`, written by `PUT /api/v1/settings/enforce_scope_strictly`; `serve` enforces strictly when it is `true` or `--enforce-scope-strictly` is given |
| `proxy_bind` | the proxy's bound address, recorded by `serve` at startup so `crosswire browser` can find it |

Intercept rules and the intercept on/off state are not stored; intercept starts disabled with default rules on every run.

`tokens` (`WITHOUT ROWID`), API tokens:

| column | type | notes |
|---|---|---|
| `id` | TEXT PRIMARY KEY | token id, 26-character ULID |
| `name` | TEXT NOT NULL | human label |
| `scopes` | TEXT NOT NULL | JSON array of kebab-case scope names (`read`, `send`, `admin`) |
| `hash` | BLOB NOT NULL UNIQUE | SHA-256 of the token's 32 secret bytes |
| `created_at` | INTEGER NOT NULL | unix milliseconds |
| `revoked_at` | INTEGER | when revoked, else NULL |
| `last_used_at` | INTEGER | last successful lookup, refreshed at most once per minute; NULL until first use |

A token's secret is `cwt_` followed by 32 random bytes in base64url (no padding). Only the SHA-256 of those 32 raw bytes is stored, in `hash`; the secret is shown once at creation and is not recoverable. Lookup is by `hash` and matches only tokens with `revoked_at` NULL; the API layer does the constant-time compare of the presented hash.

`drafts` (`WITHOUT ROWID`), one saved repeater draft per tab:

| column | type | notes |
|---|---|---|
| `tab_id` | TEXT PRIMARY KEY | repeater tab id, 26-character ULID |
| `json` | TEXT NOT NULL | the draft payload, opaque JSON chosen by the UI |
| `updated_at` | INTEGER NOT NULL | last save, unix milliseconds |

#### Tags and notes

An exchange's `tags` and `notes` can be edited after it is committed. The control connection updates both the denormalised column (`exchanges.tags`, a JSON array string or `''` when empty; `exchanges.notes`) and the matching field inside the `json` document, in one transaction, so the list filters and `Store::get` stay consistent. When the tags change it also replaces the exchange's `exchange_tags` rows (delete then insert) in the same transaction. The tag list filter matches a tag exactly against the normalized `exchange_tags` side table (`ulid IN (SELECT ulid FROM exchange_tags WHERE tag = ?)`); the `tags` JSON column is kept only for row display and `Store::get`.

### Indexes

From `0001_init.sql`:

- `exchanges_created_at` on `exchanges (created_at)`
- `exchanges_req_host` on `exchanges (req_host)`
- `exchanges_resp_status` on `exchanges (resp_status)`
- `exchanges_namespace_created_at` on `exchanges (namespace, created_at)`

From `0003_history_indexes.sql`, for the structured history queries (§13). Each trails `ulid`, so a `<column> = ? ORDER BY ulid DESC` keyset pages inside the one index (filter and order both served, no separate sort):

- `idx_exchanges_host` on `exchanges (req_host, ulid)` — host query.
- `idx_exchanges_method_status` on `exchanges (req_method, resp_status, ulid)` — method + status query.
- `idx_exchanges_resp_len` on `exchanges (resp_len, ulid)` — response-size query.
- `idx_exchanges_scope` on `exchanges (scope_state, ulid)` — scope query.

From `0004_tag_index.sql`, the normalized tag index:

- `exchange_tags` (`WITHOUT ROWID`), `(ulid TEXT, tag TEXT, PRIMARY KEY (tag, ulid))` — one row per `(exchange, tag)`. The `(tag, ulid)` primary key is a `tag → ulids` range already ordered for the newest-first keyset, so the tag query searches it by its primary key (`SEARCH exchange_tags USING PRIMARY KEY (tag=?)`) instead of scanning every row's `json_each`. The writer fills it in the ingest transaction and the control connection replaces an exchange's rows when its tags change, both in lockstep with the `tags` column.

## Lock file

`project.lock` is held with an exclusive, non-blocking `flock` for the lifetime of the open `Project`. The lock is unix-only in v1 (the crate does not compile elsewhere); Windows support is a later task. A second open fails with `Locked`. The file holds three lines:

```
<pid>
<start unix seconds>
clean=0|1
```

Opening writes `clean=0` (and `Store::open` writes it again); only `Store::close` writes `clean=1`. If the previous contents say `clean=0`, the previous run died, so `Project::open` runs `PRAGMA integrity_check` and `Store::open`, before replaying the journal, writes a `gap` marker from `last_commit_watermark` to the restart (detail `unclean shutdown`).

## Pack record

Content is stored in `packs/{id:06}.pack`, append-only. Each record:

```
magic       4 bytes   "CWPK"
clen        4 bytes   u32 little-endian, length of the zstd frame
sha256     32 bytes   SHA-256 of the raw (uncompressed) bytes
frame    clen bytes   one zstd frame
```

The `blobs` row points at the record start with `offset` and `len` (the whole record). Reading verifies the SHA-256, and refuses a frame that does not declare its content size or declares more than 64 MiB. There is no compression-ratio limit on packs: the store compressed them itself, and the size cap bounds memory (a 2 MiB run of zeros is an 83-byte frame and reads back). A pack rotates to a new file at 256 MiB. Only bytes up to `packs.committed_len` are trusted. The writer opens a pack for appending at its trusted end (`committed_len`, or the end of the last blob it references if later) and cuts any bytes past it, so records appended by a transaction that failed or rolled back never end up inside `committed_len`; a pack file left by a rolled-back rotation is cut to empty.

## Write path

One writer thread owns the writer connection. Exchanges and submitted markers reach it through a bounded queue (10,000 entries and 64 MiB, an exchange costing its message bytes plus 1 KiB, a marker one entry and its JSON length). A marker is not spilled to the journal: when the queue is full it is refused (`full`). It commits a batch when 1,000 exchanges are waiting or 250 ms after the first one arrived. Per batch, in order:

1. `BEGIN IMMEDIATE`.
2. A pending `gap` marker (exchanges were dropped): from the earliest dropped id (or one second before the first drop, if earlier) to now. Then the markers submitted since the last commit.
3. For each exchange not already stored: its bodies (inline or appended to a pack), then its `exchanges` row.
4. `last_committed_ulid` advances to the greatest id in the batch; `last_commit_watermark` is rewritten.
5. The current pack is fsynced; `packs.committed_len` is raised to the end of the last record written to each pack.
6. `COMMIT`. If the batch came from journal files, the WAL is fsynced and only then are the files deleted. Then the `committed` event is sent, then one `marker` event per marker row written.

A failure caused by one exchange (an oversized body, a JSON or constraint error) moves that exchange to `journal/bad/`, records a gap, and commits the rest. Any other failure (disk full, I/O error) is treated as the database being unavailable: queued exchanges are written back to `journal/` (held in memory if that fails too), journal files stay, one `error` event is sent per failure streak, and the writer retries with a backoff of 250 ms doubling to 30 s.

`Store::close` drains the queue and the journal. Dropping a `Store` without `close` drains only the in-memory queue; the journal waits for the next open.

`PRAGMA wal_checkpoint(PASSIVE)` runs every 30 s or when the WAL passes 64 MiB, followed by an fsync of the WAL; `close` ends with `wal_checkpoint(TRUNCATE)`.

## Journal

`journal/` holds exchanges that were accepted but are not in the database yet. Each is one file, `journal/<ulid>.json`: the exchange as a single JSON `Exchange` (the `crosswire-model` schema, bodies inline, `state` as submitted), mode 0600 in a 0700 directory. A `.json` file is always complete: the writer creates `journal/<ulid>.json.tmp`, writes and fsyncs it, renames it, and fsyncs the directory.

Two things put a file there. When the queue has no room, the sink spills the exchange before `submit` returns, and the journal is capped at 1 GiB (the sum of file sizes); past that the exchange is dropped, one `error` event is sent per episode, and the writer's next commit writes a `gap` marker. When the database is unavailable (disk full, I/O error), the writer writes the queued exchanges back to the journal; those are not counted against the cap, because they were accepted already.

The writer commits journal files oldest ULID first whenever the queue is below a quarter full, at most 1,000 files and 64 MiB of files (at least one) per batch, and all of them on every open. A committed batch's files are deleted only after the WAL is fsynced. Replay is idempotent: an id already in `exchanges` is skipped and its file deleted, but it is still listed in that batch's `committed` event, so a consumer can see the same id in `committed` twice and should deduplicate. Replay can commit ids older than rows already committed (ids are creation time), so a reader paging `list` by ULID keyset can miss them; to see every exchange, tail `committed` events. Leftover `.tmp` files were never acknowledged and are deleted on open.

`journal/bad/` (also 0700) holds what could not be committed, kept for inspection and never replayed. Two kinds land there, both named `<ulid>.json`: a journal file that cannot be parsed (a symlink, directory, FIFO or other non-regular file, larger than 256 MiB, or not valid `Exchange` JSON) or cannot be committed, moved as it is; and a queued exchange that cannot be stored (for example a body over 64 MiB), written there with an fsync. Each quarantine sends an `error` event and records a `gap` whose window starts at the exchange's ULID (for a file, the ULID in its name, or now minus one second if the name is not a ULID), so every loss has a gap row. A journal file that fails to read with an I/O error that may pass (too many open files, `EIO`, permission denied) is not quarantined: it stays in `journal/`, one `error` event is sent, and the writer backs off (250 ms doubling to 30 s per attempt) and reads it again. After 10 such retries (about two minutes) it is treated as unreadable: quarantined with a `gap`.

## Audit log

`audit.jsonl` (mode 0600, opened append-only and without following symlinks) holds one JSON object per line with six fields: `ts` (unix milliseconds), `ulid` (a fresh id for the row), `actor`, `action`, `target` and `result` (strings chosen by the caller). Each row is `fdatasync`ed before the call returns. When a row would take a non-empty file past 64 MiB, the file is synced and renamed to `audit.jsonl.1` (replacing any previous one), and a new `audit.jsonl` is created and its directory entry synced. Only one rotated generation is kept.
