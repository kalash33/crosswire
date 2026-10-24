//! Schema migrations.

use crate::error::StoreError;
use rusqlite::Connection;

/// The `application_id` of a Crosswire project (`CWPR`).
pub(crate) const APPLICATION_ID: i64 = 0x4357_5052;
/// The schema version this build reads and writes.
pub(crate) const USER_VERSION: u32 = 2;

const INIT: &str = include_str!("../migrations/0001_init.sql");
const SIDECAR: &str = include_str!("../migrations/0002_sidecar.sql");

/// Creates a fresh project at [`USER_VERSION`]: applies every migration and
/// stamps `application_id` and `user_version`, in one transaction.
pub(crate) fn init(conn: &mut Connection) -> Result<(), StoreError> {
    let tx = conn.transaction()?;
    tx.execute_batch(INIT)?;
    tx.execute_batch(SIDECAR)?;
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
    if version != USER_VERSION {
        return Err(StoreError::Corrupt(format!(
            "no migration path from version {from}"
        )));
    }
    Ok(())
}
