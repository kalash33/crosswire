//! Append-only zstd pack files and the record codec shared by reads and the sweep.
//!
//! Record layout (see `docs/format.md`): `CWPK`, `u32` LE frame length, the
//! SHA-256 of the raw bytes, then one zstd frame.

use crate::error::StoreError;
use rusqlite::Transaction;
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub(crate) const MAGIC: &[u8; 4] = b"CWPK";
/// Magic, frame length and hash.
pub(crate) const HEADER_LEN: u64 = 40;
/// Packs rotate to a new file once a record would pass this size.
pub(crate) const PACK_LIMIT: u64 = 256 * 1024 * 1024;
/// The most raw bytes one record may decompress to.
pub(crate) const MAX_RAW: u64 = 64 * 1024 * 1024;
/// The most compressed bytes one record may carry (incompressible `MAX_RAW` plus overhead).
pub(crate) const MAX_FRAME: u64 = MAX_RAW + 1024 * 1024;
/// Declared sizes up to this are exempt from the ratio check (cap-bounded anyway).
const RATIO_FLOOR: u64 = 1024 * 1024;
const MAX_RATIO: u64 = 100;
const LEVEL: i32 = 3;

/// The file of pack `id` inside project directory `dir`.
pub(crate) fn pack_file(dir: &Path, id: u32) -> PathBuf {
    dir.join("packs").join(format!("{id:06}.pack"))
}

/// Where a compressed body lives in a pack.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct BlobRef {
    /// SHA-256 of the raw bytes.
    pub hash: [u8; 32],
    /// The pack holding the record.
    pub pack_id: u32,
    /// Offset of the record start in the pack.
    pub offset: u64,
    /// Length of the compressed zstd frame (the record is 40 bytes longer).
    pub len: u32,
    /// Length of the raw bytes.
    pub raw_len: u64,
}

impl BlobRef {
    /// Length of the whole record, as stored in `blobs.len`.
    #[must_use]
    pub fn record_len(&self) -> u64 {
        HEADER_LEN + u64::from(self.len)
    }
}

/// Appends records to the current pack and rotates at 256 MiB.
#[derive(Debug)]
pub struct PackWriter {
    packs_dir: PathBuf,
    file: File,
    id: u32,
    offset: u64,
    limit: u64,
    poisoned: bool,
    #[cfg(test)]
    fail_write_after: Option<usize>,
}

impl PackWriter {
    /// Opens the newest pack of `project` for appending, creating pack 1 in
    /// an empty project. Run [`crate::sweep`] first so a torn tail is gone.
    pub fn open(project: &mut crate::Project) -> Result<PackWriter, StoreError> {
        let dir = project.dir().to_path_buf();
        let max: Option<u32> =
            project
                .writer_mut()
                .query_row("SELECT max(id) FROM packs", [], |r| r.get(0))?;
        let id = match max {
            Some(id) => id,
            None => {
                project
                    .writer_mut()
                    .execute("INSERT INTO packs (id, committed_len) VALUES (1, 0)", [])?;
                1
            }
        };
        let (file, offset) = open_append(&pack_file(&dir, id))?;
        Ok(PackWriter {
            packs_dir: dir,
            file,
            id,
            offset,
            limit: PACK_LIMIT,
            poisoned: false,
            #[cfg(test)]
            fail_write_after: None,
        })
    }

    /// Lowers the rotation size (tests only).
    #[doc(hidden)]
    pub fn set_rotate_limit(&mut self, limit: u64) {
        self.limit = limit;
    }

    /// Compresses `raw` (zstd level 3) and appends it as one record.
    ///
    /// Rotates first if the record would pass 256 MiB, inserting the new
    /// `packs` row through `tx`. If `tx` is rolled back after a rotation, the
    /// caller must drop this writer and reopen it.
    pub fn append(&mut self, tx: &Transaction<'_>, raw: &[u8]) -> Result<BlobRef, StoreError> {
        if self.poisoned {
            return Err(StoreError::Io(std::io::Error::other(
                "pack writer poisoned; reopen",
            )));
        }
        let raw_len = raw.len() as u64;
        if raw_len > MAX_RAW {
            return Err(StoreError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "body exceeds the 64 MiB storage limit",
            )));
        }
        let frame = zstd::bulk::compress(raw, LEVEL)?;
        let len = u32::try_from(frame.len()).map_err(|_| {
            StoreError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "frame too large",
            ))
        })?;
        let hash: [u8; 32] = Sha256::digest(raw).into();
        let record_len = HEADER_LEN + u64::from(len);
        if self.offset > 0 && self.offset + record_len > self.limit {
            self.rotate(tx)?;
        }
        let mut rec = Vec::with_capacity(frame.len() + HEADER_LEN as usize);
        rec.extend_from_slice(MAGIC);
        rec.extend_from_slice(&len.to_le_bytes());
        rec.extend_from_slice(&hash);
        rec.extend_from_slice(&frame);
        // A rolled-back rotation must not leave records in a pack without a row.
        tx.execute(
            "INSERT OR IGNORE INTO packs (id, committed_len) VALUES (?1, 0)",
            [self.id],
        )?;
        if let Err(e) = self.write_record(&rec) {
            // Drop the torn bytes so the next record lands at `self.offset`.
            let undone = self
                .file
                .set_len(self.offset)
                .and_then(|()| self.file.sync_all());
            if undone.is_err() {
                self.poisoned = true;
            }
            return Err(e.into());
        }
        let blob = BlobRef {
            hash,
            pack_id: self.id,
            offset: self.offset,
            len,
            raw_len,
        };
        self.offset += record_len;
        Ok(blob)
    }

    fn write_record(&mut self, rec: &[u8]) -> std::io::Result<()> {
        #[cfg(test)]
        if let Some(n) = self.fail_write_after.take() {
            self.file.write_all(&rec[..n.min(rec.len())])?;
            return Err(std::io::Error::other("injected write failure"));
        }
        self.file.write_all(rec)
    }

    fn rotate(&mut self, tx: &Transaction<'_>) -> Result<(), StoreError> {
        self.file.sync_all()?;
        let id = self.id + 1;
        let (file, offset) = open_append(&pack_file(&self.packs_dir, id))?;
        tx.execute(
            "INSERT OR IGNORE INTO packs (id, committed_len) VALUES (?1, 0)",
            [id],
        )?;
        self.file = file;
        self.id = id;
        self.offset = offset;
        Ok(())
    }

    /// Flushes the current pack to disk.
    pub fn fsync(&mut self) -> Result<(), StoreError> {
        self.file.sync_all()?;
        Ok(())
    }

    /// The id of the pack being appended to.
    #[must_use]
    pub fn pack_id(&self) -> u32 {
        self.id
    }

    /// The end of the current pack: the value to pass to
    /// [`crate::Project::set_pack_committed_len`] once the batch is fsynced
    /// and its transaction commits.
    #[must_use]
    pub fn committed_len(&self) -> u64 {
        self.offset
    }
}

fn open_append(path: &Path) -> Result<(File, u64), StoreError> {
    check_pack_path(path, true)?;
    let mut o = OpenOptions::new();
    o.append(true).create(true);
    harden_open(&mut o);
    let file = o.open(path)?;
    let len = file.metadata()?.len();
    Ok((file, len))
}

/// Opens an existing pack for reading (and writing when `write`), refusing symlinks.
pub(crate) fn open_pack(path: &Path, write: bool) -> Result<File, StoreError> {
    check_pack_path(path, false)?;
    let mut o = OpenOptions::new();
    o.read(true).write(write);
    harden_open(&mut o);
    Ok(o.open(path)?)
}

/// `packs/` must be a real directory and the pack a regular file (or absent
/// when `allow_missing`); a hostile project must not redirect us elsewhere.
fn check_pack_path(path: &Path, allow_missing: bool) -> Result<(), StoreError> {
    let refuse = || StoreError::Corrupt("pack path is not a regular file".into());
    if let Some(dir) = path.parent() {
        let m = std::fs::symlink_metadata(dir)?;
        if !m.file_type().is_dir() {
            return Err(refuse());
        }
    }
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_file() => Ok(()),
        Ok(_) => Err(refuse()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && allow_missing => Ok(()),
        Err(e) => Err(e.into()),
    }
}

pub(crate) fn harden_open(o: &mut OpenOptions) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600)
            .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32);
    }
    #[cfg(not(unix))]
    let _ = o;
}

/// A record that decoded and verified.
pub(crate) struct Record {
    pub hash: [u8; 32],
    pub raw: Vec<u8>,
    /// Header plus frame.
    pub total_len: u64,
}

/// Why a record did not decode.
#[derive(Debug)]
pub(crate) enum BadRecord {
    /// The bytes are not a valid record (torn, garbage, over a limit).
    Bad(&'static str),
    /// The file could not be read.
    Io(std::io::Error),
}

impl From<std::io::Error> for BadRecord {
    fn from(e: std::io::Error) -> Self {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            BadRecord::Bad("record truncated")
        } else {
            BadRecord::Io(e)
        }
    }
}

/// Reads and verifies the record at `offset`; `file_len` bounds every read.
///
/// Allocation is bounded: the frame buffer is at most `MAX_FRAME` and the
/// decompressed output at most `MAX_RAW`, both checked before allocating.
pub(crate) fn read_record(
    file: &mut File,
    offset: u64,
    file_len: u64,
) -> Result<Record, BadRecord> {
    let remaining = file_len.saturating_sub(offset);
    if remaining < HEADER_LEN {
        return Err(BadRecord::Bad("record header truncated"));
    }
    file.seek(SeekFrom::Start(offset))?;
    let mut head = [0u8; HEADER_LEN as usize];
    file.read_exact(&mut head)?;
    if &head[..4] != MAGIC {
        return Err(BadRecord::Bad("bad magic"));
    }
    let clen = u64::from(u32::from_le_bytes([head[4], head[5], head[6], head[7]]));
    if clen > MAX_FRAME {
        return Err(BadRecord::Bad("frame length over limit"));
    }
    if clen > remaining - HEADER_LEN {
        return Err(BadRecord::Bad("record extends past end of file"));
    }
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&head[8..]);
    let mut frame = vec![0u8; clen as usize];
    file.read_exact(&mut frame)?;
    let raw = decompress_bounded(&frame)?;
    if Sha256::digest(&raw).as_slice() != hash {
        return Err(BadRecord::Bad("body hash mismatch"));
    }
    Ok(Record {
        hash,
        raw,
        total_len: HEADER_LEN + clen,
    })
}

/// Decompresses one frame, refusing declared sizes over 64 MiB or 100:1.
fn decompress_bounded(frame: &[u8]) -> Result<Vec<u8>, BadRecord> {
    let declared = match zstd::zstd_safe::get_frame_content_size(frame) {
        Ok(Some(n)) => n,
        Ok(None) => return Err(BadRecord::Bad("frame has no content size")),
        Err(_) => return Err(BadRecord::Bad("not a zstd frame")),
    };
    if declared > MAX_RAW {
        return Err(BadRecord::Bad("decompressed size over limit"));
    }
    if declared > RATIO_FLOOR && declared > MAX_RATIO * frame.len() as u64 {
        return Err(BadRecord::Bad("compression ratio over limit"));
    }
    // `capacity` is the hard cap: a frame that expands past it is an error.
    let cap = usize::try_from(declared).map_err(|_| BadRecord::Bad("size over limit"))?;
    let out = zstd::bulk::decompress(frame, cap)
        .map_err(|_| BadRecord::Bad("frame does not decompress"))?;
    if out.len() as u64 != declared {
        return Err(BadRecord::Bad("frame size mismatch"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Project;

    #[test]
    fn failed_write_is_undone_and_next_append_keeps_its_offset() {
        let tmp = tempfile::tempdir().unwrap();
        let mut p = Project::create(&tmp.path().join("p")).unwrap();
        let mut w = PackWriter::open(&mut p).unwrap();
        let tx = p.writer_mut().transaction().unwrap();
        let a = w.append(&tx, &vec![1u8; 30_000]).unwrap();
        let before = w.committed_len();
        w.fail_write_after = Some(25);
        assert!(w.append(&tx, &vec![2u8; 30_000]).is_err());
        assert_eq!(w.committed_len(), before);
        let path = pack_file(tmp.path().join("p").as_path(), 1);
        assert_eq!(std::fs::metadata(&path).unwrap().len(), before);
        let b = w.append(&tx, &vec![3u8; 30_000]).unwrap();
        assert_eq!(b.offset, a.offset + a.record_len());
        assert_eq!(b.offset, before);
    }

    #[test]
    fn poisoned_writer_refuses_appends() {
        let tmp = tempfile::tempdir().unwrap();
        let mut p = Project::create(&tmp.path().join("p")).unwrap();
        let mut w = PackWriter::open(&mut p).unwrap();
        w.poisoned = true;
        let tx = p.writer_mut().transaction().unwrap();
        assert!(w.append(&tx, b"x").is_err());
    }
}
