-- Crosswire project schema, version 2 (additive). Documented in docs/format.md.
-- The sidecar control path: UI/server settings, API tokens and repeater drafts.
-- These tables are written by the control connection, never by the writer thread.

CREATE TABLE settings (
    key        TEXT PRIMARY KEY,
    value      TEXT NOT NULL,
    updated_at INTEGER NOT NULL
) WITHOUT ROWID;

CREATE TABLE tokens (
    id           TEXT PRIMARY KEY,
    name         TEXT NOT NULL,
    scopes       TEXT NOT NULL,
    hash         BLOB NOT NULL UNIQUE,
    created_at   INTEGER NOT NULL,
    revoked_at   INTEGER,
    last_used_at INTEGER
) WITHOUT ROWID;

CREATE TABLE drafts (
    tab_id     TEXT PRIMARY KEY,
    json       TEXT NOT NULL,
    updated_at INTEGER NOT NULL
) WITHOUT ROWID;
