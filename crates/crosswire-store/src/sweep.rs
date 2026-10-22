//! Startup sweep: truncate torn pack tails and repair what pointed into them.

use crate::error::StoreError;
use crate::packs::{read_record, BadRecord};
use crate::Project;
use crosswire_model::{Marker, MarkerKind};
use rusqlite::Transaction;
use std::collections::BTreeSet;

/// What [`sweep`] did.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct SweepReport {
    /// Pack rows examined.
    pub packs_checked: u64,
    /// Records past `committed_len` that decoded and verified.
    pub records_validated: u64,
    /// Bytes cut from pack tails.
    pub truncated_bytes: u64,
    /// Exchanges set to `state = 'error'` with a gap marker.
    pub exchanges_marked: u64,
    /// Bodies whose exchange row does not exist (no marker written).
    pub missing_exchanges: u64,
    /// Unreferenced blob rows deleted.
    pub orphans_deleted: u64,
}

/// Validates every pack from its `committed_len` to the end of the file.
///
/// `committed_len` is the only trust boundary: bytes before it are not read.
/// The first record that is truncated, has a bad magic or length, exceeds the
/// decompression limits or fails its hash ends the valid region; the file is
/// cut there. Blobs that end past the cut are deleted, each exchange that used
/// one is marked `error` with a `Gap` marker, and unreferenced zero-refcount
/// blobs are removed. Running it again changes nothing.
pub fn sweep(project: &mut Project) -> Result<SweepReport, StoreError> {
    let mut report = SweepReport::default();
    let packs: Vec<(u32, u64)> = {
        let mut st = project
            .writer_mut()
            .prepare("SELECT id, committed_len FROM packs ORDER BY id")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?;
        let mut v = Vec::new();
        for row in rows {
            let (id, committed) = row?;
            let id = u32::try_from(id)
                .map_err(|_| StoreError::Corrupt("pack id out of range".into()))?;
            v.push((id, u64::try_from(committed).unwrap_or(0)));
        }
        v
    };
    let mut cuts: Vec<(u32, u64)> = Vec::new();
    for (id, committed) in packs {
        report.packs_checked += 1;
        let path = project.pack_path(id);
        let cut = match crate::packs::open_pack(&path, true) {
            Ok(mut file) => {
                let file_len = file.metadata()?.len();
                let mut pos = committed.min(file_len);
                while pos < file_len {
                    match read_record(&mut file, pos, file_len) {
                        Ok(rec) => {
                            report.records_validated += 1;
                            pos += rec.total_len;
                        }
                        Err(BadRecord::Bad(_)) => break,
                        Err(BadRecord::Io(e)) => return Err(e.into()),
                    }
                }
                if pos < file_len {
                    file.set_len(pos)?;
                    file.sync_all()?;
                    report.truncated_bytes += file_len - pos;
                }
                pos
            }
            Err(StoreError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => 0,
            Err(e) => return Err(e),
        };
        // Always reconcile the index with the file, so a crash between the
        // truncate and the database cleanup is repaired by the next sweep.
        cuts.push((id, cut));
    }
    let tx = project.writer_mut().transaction()?;
    let mut lost: BTreeSet<String> = BTreeSet::new();
    for (id, cut) in cuts {
        lose_blobs(&tx, id, cut, &mut lost)?;
    }
    for ulid in &lost {
        let changed = tx.execute(
            "UPDATE exchanges SET state = 'error' WHERE ulid = ?1",
            [ulid],
        )?;
        if changed == 0 {
            report.missing_exchanges += 1;
            continue;
        }
        let marker = Marker::new(MarkerKind::Gap, "pack torn");
        let kind = serde_json::to_value(marker.kind())
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_else(|| "gap".into());
        tx.execute(
            "INSERT INTO markers (ulid, kind, from_ulid, detail) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![marker.id().to_string(), kind, ulid, marker.detail()],
        )?;
        report.exchanges_marked += 1;
    }
    report.orphans_deleted = tx.execute(
        "DELETE FROM blobs WHERE refcount <= 0 \
         AND hash NOT IN (SELECT blob_hash FROM bodies WHERE blob_hash IS NOT NULL)",
        [],
    )? as u64;
    tx.commit()?;
    Ok(report)
}

/// Deletes the blobs of pack `id` that end past `cut`, noting who used them.
fn lose_blobs(
    tx: &Transaction<'_>,
    id: u32,
    cut: u64,
    lost: &mut BTreeSet<String>,
) -> Result<(), StoreError> {
    let cut = i64::try_from(cut).unwrap_or(i64::MAX);
    let hashes: Vec<Vec<u8>> = {
        let mut st = tx.prepare(
            "SELECT hash FROM blobs WHERE pack_id = ?1 AND offset >= ?2 OR offset + len > ?2",
        )?;
        let rows = st.query_map(rusqlite::params![id, cut], |r| r.get(0))?;
        rows.collect::<Result<_, _>>()?
    };
    for hash in hashes {
        let mut st = tx.prepare("SELECT ulid FROM bodies WHERE blob_hash = ?1")?;
        let users = st.query_map([hash.as_slice()], |r| r.get::<_, String>(0))?;
        for u in users {
            lost.insert(u?);
        }
        tx.execute("DELETE FROM blobs WHERE hash = ?1", [hash.as_slice()])?;
    }
    Ok(())
}
