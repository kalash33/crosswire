//! Schema migrations.

use crate::error::StoreError;
use rusqlite::Connection;

/// The `application_id` of a Crosswire project (`CWPR`).
pub(crate) const APPLICATION_ID: i64 = 0x4357_5052;
/// The schema version this build reads and writes.
pub(crate) const USER_VERSION: u32 = 3;

const INIT: &str = include_str!("../migrations/0001_init.sql");
const SIDECAR: &str = include_str!("../migrations/0002_sidecar.sql");
const HISTORY_INDEXES: &str = include_str!("../migrations/0003_history_indexes.sql");

/// Creates a fresh project at [`USER_VERSION`]: applies every migration and
/// stamps `application_id` and `user_version`, in one transaction.
pub(crate) fn init(conn: &mut Connection) -> Result<(), StoreError> {
    let tx = conn.transaction()?;
    tx.execute_batch(INIT)?;
    tx.execute_batch(SIDECAR)?;
    tx.execute_batch(HISTORY_INDEXES)?;
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
    if version != USER_VERSION {
        return Err(StoreError::Corrupt(format!(
            "no migration path from version {from}"
        )));
    }
    Ok(())
}
