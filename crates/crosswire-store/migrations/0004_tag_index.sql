-- Crosswire project schema, version 4 (additive). Documented in docs/format.md.
-- A normalized side table for the `tag = ?` history query (spec §13). The
-- denormalized `exchanges.tags` JSON column stays (it feeds row display and
-- `Store::get`); this table is the queryable index of it. The `(tag, ulid)`
-- primary key is a `tag -> ulids` range already ordered for the newest-first
-- keyset, so `WHERE ulid IN (SELECT ulid FROM exchange_tags WHERE tag = ?)`
-- searches it by its primary key instead of scanning every row's `json_each`.
-- Idempotent: `IF NOT EXISTS` lets `init` (fresh, empty) and `migrate`
-- (existing, then backfilled in the same tx) apply the same statement.

CREATE TABLE IF NOT EXISTS exchange_tags (
    ulid TEXT NOT NULL,
    tag  TEXT NOT NULL,
    PRIMARY KEY (tag, ulid)
) WITHOUT ROWID;
