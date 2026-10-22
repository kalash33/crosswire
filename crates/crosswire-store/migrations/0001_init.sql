-- Crosswire project schema, version 1. Documented in docs/format.md.

CREATE TABLE exchanges (
    ulid        TEXT PRIMARY KEY,
    namespace   TEXT NOT NULL,
    state       TEXT NOT NULL,
    scope_state TEXT NOT NULL,
    created_at  INTEGER NOT NULL,
    json        TEXT NOT NULL,
    req_method  TEXT,
    req_host    TEXT,
    req_path    TEXT,
    resp_status INTEGER,
    resp_len    INTEGER,
    tags        TEXT NOT NULL DEFAULT '',
    notes       TEXT NOT NULL DEFAULT ''
) WITHOUT ROWID;

CREATE TABLE bodies (
    ulid      TEXT NOT NULL,
    side      TEXT NOT NULL CHECK (side IN ('req', 'resp')),
    inline    BLOB,
    blob_hash BLOB,
    len       INTEGER NOT NULL,
    PRIMARY KEY (ulid, side),
    CHECK (inline IS NULL OR blob_hash IS NULL)
) WITHOUT ROWID;

CREATE TABLE blobs (
    hash     BLOB PRIMARY KEY,
    pack_id  INTEGER NOT NULL,
    offset   INTEGER NOT NULL,
    len      INTEGER NOT NULL,
    refcount INTEGER NOT NULL DEFAULT 1
) WITHOUT ROWID;

CREATE TABLE packs (
    id            INTEGER PRIMARY KEY,
    committed_len INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE markers (
    ulid      TEXT PRIMARY KEY,
    kind      TEXT NOT NULL,
    from_ulid TEXT,
    to_ulid   TEXT,
    detail    TEXT NOT NULL DEFAULT ''
) WITHOUT ROWID;

CREATE TABLE meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
) WITHOUT ROWID;

CREATE INDEX exchanges_created_at ON exchanges (created_at);
CREATE INDEX exchanges_req_host ON exchanges (req_host);
CREATE INDEX exchanges_resp_status ON exchanges (resp_status);
CREATE INDEX exchanges_namespace_created_at ON exchanges (namespace, created_at);

INSERT INTO meta (key, value) VALUES ('last_committed_ulid', '');
