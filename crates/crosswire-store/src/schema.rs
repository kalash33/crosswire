//! Schema migrations.

use crate::error::StoreError;
use rusqlite::Connection;

/// The `application_id` of a Crosswire project (`CWPR`).
pub(crate) const APPLICATION_ID: i64 = 0x4357_5052;
/// The schema version this build reads and writes.
pub(crate) const USER_VERSION: u32 = 1;

const INIT: &str = include_str!("../migrations/0001_init.sql");

/// Applies migration 1 and stamps `application_id` and `user_version`, in one transaction.
pub(crate) fn init(conn: &mut Connection) -> Result<(), StoreError> {
    let tx = conn.transaction()?;
    tx.execute_batch(INIT)?;
    tx.pragma_update(None, "application_id", APPLICATION_ID)?;
    tx.pragma_update(None, "user_version", USER_VERSION)?;
    tx.commit()?;
    Ok(())
}
