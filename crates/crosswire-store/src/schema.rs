//! Schema migrations.

use crate::error::StoreError;
use rusqlite::Connection;

/// The `application_id` of a Crosswire project (`CWPR`).
pub(crate) const APPLICATION_ID: i64 = 0x4357_5052;
/// The schema version this build reads and writes.
pub(crate) const USER_VERSION: u32 = 6;

/// Rows backfilled into `exchange_tags` per statement during the v3→v4
/// migration, so a million-row project is not one unbounded insert.
const TAG_BACKFILL_CHUNK: usize = 1000;

const INIT: &str = include_str!("../migrations/0001_init.sql");
const SIDECAR: &str = include_str!("../migrations/0002_sidecar.sql");
const HISTORY_INDEXES: &str = include_str!("../migrations/0003_history_indexes.sql");
const TAG_INDEX: &str = include_str!("../migrations/0004_tag_index.sql");
const METHOD_STATUS_ORDER: &str = include_str!("../migrations/0005_method_status_order.sql");

/// Creates a fresh project at [`USER_VERSION`]: applies every migration and
/// stamps `application_id` and `user_version`, in one transaction.
pub(crate) fn init(conn: &mut Connection) -> Result<(), StoreError> {
    let tx = conn.transaction()?;
    tx.execute_batch(INIT)?;
    tx.execute_batch(SIDECAR)?;
    tx.execute_batch(HISTORY_INDEXES)?;
    tx.execute_batch(TAG_INDEX)?;
    tx.execute_batch(METHOD_STATUS_ORDER)?;
    tx.pragma_update(None, "application_id", APPLICATION_ID)?;
    tx.pragma_update(None, "user_version", USER_VERSION)?;
    tx.commit()?;
    Ok(())
}

/// Migrates a project from `from` up to [`USER_VERSION`].
///
/// Each step runs in its own transaction that also advances `user_version`,
/// so a crash mid-upgrade leaves a consistent earlier version. `from` is
/// trusted to be in `1..USER_VERSION` (the caller checked the range and took
/// the pre-migration snapshot).
pub(crate) fn migrate(conn: &mut Connection, from: u32) -> Result<(), StoreError> {
    let mut version = from;
    if version == 1 {
        let tx = conn.transaction()?;
        tx.execute_batch(SIDECAR)?;
        tx.pragma_update(None, "user_version", 2)?;
        tx.commit()?;
        version = 2;
    }
    if version == 2 {
        let tx = conn.transaction()?;
        tx.execute_batch(HISTORY_INDEXES)?;
        // Give the planner statistics for the new indexes. Without them, an
        // open-ended range like `resp_len >= ?` over a `WITHOUT ROWID` table is
        // served by a primary-key (ulid-ordered) scan — the very full scan the
        // `resp_len` index is meant to replace at a million rows — because the
        // planner assumes a range matches a quarter of the rows. `ANALYZE` on
        // the existing, populated project records the real (skewed) resp_len
        // distribution so the selective range uses `idx_exchanges_resp_len`.
        // A fresh project (`init`) is empty, so it is not analysed here; it
        // gathers statistics once it holds representative data.
        tx.execute_batch("ANALYZE;")?;
        tx.pragma_update(None, "user_version", 3)?;
        tx.commit()?;
        version = 3;
    }
    if version == 3 {
        let tx = conn.transaction()?;
        tx.execute_batch(TAG_INDEX)?;
        // Backfill `exchange_tags` from the denormalized `tags` JSON of every
        // existing row, in bounded chunks so a large project is not one
        // unbounded insert. Each pass takes a chunk of still-unindexed tagged
        // rows and explodes their JSON array into `(ulid, tag)` rows; the
        // `NOT IN (SELECT ulid FROM exchange_tags)` guard makes the chunks
        // disjoint, and empty (`''`) / `'[]'` tag columns are skipped so an
        // exchange with no tags is never reselected. The whole backfill runs
        // in this step's own transaction, before `user_version` reaches 4.
        loop {
            let n = tx.execute(
                "INSERT OR IGNORE INTO exchange_tags (ulid, tag) \
                 SELECT e.ulid, j.value FROM exchanges e, json_each(e.tags) j \
                 WHERE e.ulid IN ( \
                     SELECT ulid FROM exchanges \
                     WHERE tags <> '' AND tags <> '[]' \
                       AND ulid NOT IN (SELECT ulid FROM exchange_tags) \
                     LIMIT ?1)",
                [TAG_BACKFILL_CHUNK],
            )?;
            if n == 0 {
                break;
            }
        }
        tx.pragma_update(None, "user_version", 4)?;
        tx.commit()?;
        version = 4;
    }
    if version == 4 {
        let tx = conn.transaction()?;
        tx.execute_batch(METHOD_STATUS_ORDER)?;
        // Statistics for the rebuilt index, so the planner picks it over a
        // primary-key scan on an existing, populated project.
        tx.execute_batch("ANALYZE;")?;
        tx.pragma_update(None, "user_version", 5)?;
        tx.commit()?;
        version = 5;
    }
    if version == 5 {
        // Format 6 changes the message wire format (schema_version 2), not the
        // tables, so this step only stamps the version. Projects older than 6
        // are refused at open and never reach here; the arm exists so the
        // version chain stays total.
        let tx = conn.transaction()?;
        tx.pragma_update(None, "user_version", 6)?;
        tx.commit()?;
        version = 6;
    }
    if version != USER_VERSION {
        return Err(StoreError::Corrupt(format!(
            "no migration path from version {from}"
        )));
    }
    Ok(())
}
