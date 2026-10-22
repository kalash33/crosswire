//! Body storage: small bodies inline in `SQLite`, larger ones in packs.

use crate::error::StoreError;
use crate::packs::{read_record, BadRecord, BlobRef, PackWriter, HEADER_LEN};
use rusqlite::{Connection, OptionalExtension, Transaction};
use sha2::{Digest, Sha256};
use std::path::Path;
use ulid::Ulid;

/// The `inline` and `blob_hash` columns of a `bodies` row.
type InlineOrHash = (Option<Vec<u8>>, Option<Vec<u8>>);

/// Bodies up to this many bytes are stored inline in `bodies.inline`.
pub const INLINE_MAX: usize = 16 * 1024;

/// Which message of an exchange a body belongs to.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BodySide {
    /// The request body.
    Req,
    /// The response body.
    Resp,
}

impl BodySide {
    fn as_str(self) -> &'static str {
        match self {
            BodySide::Req => "req",
            BodySide::Resp => "resp",
        }
    }
}

/// Where [`BodyStore::put`] stored a body.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BodyLocator {
    /// In the `bodies` row.
    Inline {
        /// Raw length.
        len: u64,
    },
    /// In a pack.
    Pack {
        /// The record.
        blob: BlobRef,
        /// True if an identical body was already stored (nothing was written).
        deduped: bool,
    },
}

/// Writes bodies under the writer's transaction and reads them back verified.
#[derive(Debug)]
pub struct BodyStore {
    packs: PackWriter,
}

impl BodyStore {
    /// Wraps a [`PackWriter`].
    #[must_use]
    pub fn new(packs: PackWriter) -> BodyStore {
        BodyStore { packs }
    }

    /// Stores `raw` as the `side` body of exchange `ulid` inside `tx`.
    ///
    /// At most 16 KiB goes inline; anything larger is deduplicated by
    /// SHA-256 (bumping `refcount`) or appended to the pack.
    pub fn put(
        &mut self,
        tx: &Transaction<'_>,
        ulid: Ulid,
        side: BodySide,
        raw: &[u8],
    ) -> Result<BodyLocator, StoreError> {
        let key = ulid.to_string();
        let len = raw.len() as u64;
        let sql_len =
            i64::try_from(len).map_err(|_| StoreError::Corrupt("body too large".into()))?;
        if raw.len() <= INLINE_MAX {
            tx.execute(
                "INSERT INTO bodies (ulid, side, inline, len) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![key, side.as_str(), raw, sql_len],
            )?;
            return Ok(BodyLocator::Inline { len });
        }
        let hash: [u8; 32] = Sha256::digest(raw).into();
        let existing: Option<(u32, u64, u64)> = tx
            .query_row(
                "SELECT pack_id, offset, len FROM blobs WHERE hash = ?1",
                [hash.as_slice()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let (blob, deduped) = if let Some((pack_id, offset, rec_len)) = existing {
            tx.execute(
                "UPDATE blobs SET refcount = refcount + 1 WHERE hash = ?1",
                [hash.as_slice()],
            )?;
            let blob = BlobRef {
                hash,
                pack_id,
                offset,
                len: u32::try_from(rec_len.saturating_sub(HEADER_LEN)).unwrap_or(u32::MAX),
                raw_len: len,
            };
            (blob, true)
        } else {
            let blob = self.packs.append(tx, raw)?;
            tx.execute(
                "INSERT INTO blobs (hash, pack_id, offset, len) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![
                    hash.as_slice(),
                    blob.pack_id,
                    blob.offset,
                    blob.record_len()
                ],
            )?;
            (blob, false)
        };
        tx.execute(
            "INSERT INTO bodies (ulid, side, blob_hash, len) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![key, side.as_str(), hash.as_slice(), sql_len],
        )?;
        Ok(BodyLocator::Pack { blob, deduped })
    }

    /// Flushes the current pack to disk.
    pub fn fsync(&mut self) -> Result<(), StoreError> {
        self.packs.fsync()
    }

    /// The end of the current pack (see [`PackWriter::committed_len`]).
    #[must_use]
    pub fn committed_len(&self) -> u64 {
        self.packs.committed_len()
    }

    /// The id of the pack being appended to.
    #[must_use]
    pub fn pack_id(&self) -> u32 {
        self.packs.pack_id()
    }

    /// Reads a body back, or `None` if the exchange has none on that side.
    ///
    /// Pack bodies are decompressed under a 64 MiB / 100:1 bound and their
    /// SHA-256 is checked against `blobs.hash`; a failure is
    /// [`StoreError::Corrupt`]. `dir` is the project directory; the pack file
    /// is derived from the stored id, never from a stored path.
    pub fn get(
        conn: &Connection,
        dir: &Path,
        ulid: Ulid,
        side: BodySide,
    ) -> Result<Option<Vec<u8>>, StoreError> {
        let row: Option<InlineOrHash> = conn
            .query_row(
                "SELECT inline, blob_hash FROM bodies WHERE ulid = ?1 AND side = ?2",
                rusqlite::params![ulid.to_string(), side.as_str()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((inline, hash)) = row else {
            return Ok(None);
        };
        if let Some(bytes) = inline {
            return Ok(Some(bytes));
        }
        let Some(hash) = hash else {
            return Err(StoreError::Corrupt("body row has no content".into()));
        };
        let loc: Option<(i64, i64)> = conn
            .query_row(
                "SELECT pack_id, offset FROM blobs WHERE hash = ?1",
                [hash.as_slice()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((pack_id, offset)) = loc else {
            return Err(StoreError::Corrupt("body blob missing".into()));
        };
        let (Ok(pack_id), Ok(offset)) = (u32::try_from(pack_id), u64::try_from(offset)) else {
            return Err(StoreError::Corrupt("blob location out of range".into()));
        };
        let mut file = std::fs::File::open(crate::packs::pack_file(dir, pack_id))?;
        let file_len = file.metadata()?.len();
        let rec = match read_record(&mut file, offset, file_len) {
            Ok(r) => r,
            Err(BadRecord::Bad(m)) => return Err(StoreError::Corrupt(m.into())),
            Err(BadRecord::Io(e)) => return Err(e.into()),
        };
        if rec.hash.as_slice() != hash.as_slice() {
            return Err(StoreError::Corrupt("body hash mismatch".into()));
        }
        Ok(Some(rec.raw))
    }
}
