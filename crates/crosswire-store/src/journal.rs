//! The spill journal: one fsynced JSON file per exchange the queue had no room for.
//!
//! A spill is written to `journal/<ulid>.json.tmp`, fsynced, renamed to
//! `journal/<ulid>.json` and the directory fsynced, so a `.json` file is
//! always complete. Leftover `.tmp` files were never acknowledged and are
//! deleted on open. Files that cannot parse are moved to `journal/bad/`
//! (with a gap row); files that fail with a transient I/O error stay and are
//! retried.

use crate::error::StoreError;
use crosswire_model::Exchange;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// The largest journal file read back (two 64 MiB bodies in base64 fit).
const MAX_FILE: u64 = 256 * 1024 * 1024;

/// Why a spill failed.
#[derive(Debug)]
pub(crate) enum SpillError {
    /// The journal holds `journal_cap` bytes already.
    CapExceeded,
    /// Writing the file failed.
    Io(StoreError),
}

/// Why [`Journal::read`] returned no exchange.
#[derive(Debug)]
pub(crate) enum ReadError {
    /// The file is gone (committed and deleted meanwhile); nothing to do.
    Gone,
    /// The file will never parse: quarantine it and record a gap.
    Unusable(String),
    /// Reading failed in a way that may pass: leave the file and retry.
    Transient(std::io::Error),
}

/// A journal file found by [`Journal::oldest`].
#[derive(Debug)]
pub(crate) struct Entry {
    pub(crate) path: PathBuf,
    pub(crate) len: u64,
}

/// The `journal/` directory and a running total of its size.
#[derive(Debug)]
pub(crate) struct Journal {
    dir: PathBuf,
    bytes: AtomicU64,
    cap: u64,
}

impl Journal {
    /// Opens `journal/` under the project directory, deleting unacknowledged
    /// `.tmp` files and counting the size of the rest.
    pub(crate) fn open(project_dir: &Path, cap: u64) -> Result<Journal, StoreError> {
        let dir = project_dir.join("journal");
        crate::project::make_real_dir(&dir, "journal/")?;
        let mut bytes = 0u64;
        for e in fs::read_dir(&dir)? {
            let e = e?;
            let name = e.file_name();
            let name = name.to_string_lossy();
            let meta = fs::symlink_metadata(e.path())?;
            if name.ends_with(".json.tmp") && !meta.is_dir() {
                fs::remove_file(e.path())?;
            } else if name.ends_with(".json") && meta.is_file() {
                bytes = bytes.saturating_add(meta.len());
            }
        }
        Ok(Journal {
            dir,
            bytes: AtomicU64::new(bytes),
            cap,
        })
    }

    /// Writes `ex` to a fsynced temporary file, counting it against the cap
    /// when `enforce_cap`. [`Prepared::publish`] makes it visible.
    pub(crate) fn prepare(
        &self,
        ex: &Exchange,
        enforce_cap: bool,
    ) -> Result<Prepared<'_>, SpillError> {
        let data = serde_json::to_vec(ex).map_err(|e| SpillError::Io(e.into()))?;
        let len = data.len() as u64;
        let cap = if enforce_cap { self.cap } else { u64::MAX };
        if self
            .bytes
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |b| {
                b.checked_add(len).filter(|n| *n <= cap)
            })
            .is_err()
        {
            return Err(SpillError::CapExceeded);
        }
        let name = format!("{}.json", ex.id());
        let prepared = Prepared {
            journal: self,
            tmp: self.dir.join(format!("{name}.tmp")),
            name,
            len,
            published: false,
        };
        write_synced(&prepared.tmp, &data).map_err(|e| SpillError::Io(e.into()))?;
        Ok(prepared)
    }

    /// Writes an exchange the writer could not commit back to the journal
    /// (not counted against the cap: it was accepted already).
    pub(crate) fn write_back(&self, ex: &Exchange) -> Result<(), SpillError> {
        self.prepare(ex, false)?.publish()
    }

    /// Up to `max` journal files, oldest ULID first.
    pub(crate) fn oldest(&self, max: usize) -> Result<Vec<Entry>, StoreError> {
        let mut names: Vec<String> = Vec::new();
        for e in fs::read_dir(&self.dir)? {
            let name = e?.file_name().to_string_lossy().into_owned();
            if name.ends_with(".json") {
                names.push(name);
            }
        }
        names.sort_unstable();
        let mut out = Vec::new();
        for name in names.into_iter().take(max) {
            let path = self.dir.join(name);
            // A symlink or directory is reported with length 0 and quarantined by `read`.
            let len = match fs::symlink_metadata(&path) {
                Ok(m) if m.is_file() => m.len(),
                Ok(_) => 0,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e.into()),
            };
            out.push(Entry { path, len });
        }
        Ok(out)
    }

    /// Reads and parses one journal file.
    ///
    /// A file that is not a regular file (a symlink is refused by
    /// `O_NOFOLLOW`), is over 256 MiB or is not `Exchange` JSON is
    /// [`ReadError::Unusable`]. Any other I/O error (`EMFILE`, `EIO`,
    /// `EACCES`, ...) may pass and is [`ReadError::Transient`].
    pub(crate) fn read(entry: &Entry) -> Result<Exchange, ReadError> {
        let mut o = OpenOptions::new();
        o.read(true);
        crate::packs::harden_open(&mut o);
        let mut f = o.open(&entry.path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ReadError::Gone
            } else if e.raw_os_error() == Some(rustix::io::Errno::LOOP.raw_os_error()) {
                ReadError::Unusable("a symlink".into())
            } else {
                ReadError::Transient(e)
            }
        })?;
        let meta = f.metadata().map_err(ReadError::Transient)?;
        if !meta.is_file() {
            return Err(ReadError::Unusable("not a regular file".into()));
        }
        if meta.len() > MAX_FILE {
            return Err(ReadError::Unusable("file too large".into()));
        }
        let mut data = Vec::new();
        Read::by_ref(&mut f)
            .take(MAX_FILE + 1)
            .read_to_end(&mut data)
            .map_err(ReadError::Transient)?;
        serde_json::from_slice(&data).map_err(|e| ReadError::Unusable(e.to_string()))
    }

    /// Deletes a file whose exchange is committed.
    pub(crate) fn remove(&self, entry: &Entry) -> Result<(), StoreError> {
        match fs::remove_file(&entry.path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        self.release(entry.len);
        Ok(())
    }

    /// Moves a file the writer cannot use to `journal/bad/`.
    pub(crate) fn quarantine(&self, entry: &Entry) -> Result<(), StoreError> {
        let bad = self.dir.join("bad");
        crate::project::make_real_dir(&bad, "journal/bad/")?;
        let name = entry.path.file_name().unwrap_or_default();
        fs::rename(&entry.path, bad.join(name))?;
        self.release(entry.len);
        Ok(())
    }

    /// Writes an exchange that could not be committed to `journal/bad/`.
    pub(crate) fn quarantine_exchange(&self, ex: &Exchange) -> Result<(), StoreError> {
        let bad = self.dir.join("bad");
        crate::project::make_real_dir(&bad, "journal/bad/")?;
        let data = serde_json::to_vec(ex)?;
        write_synced(&bad.join(format!("{}.json", ex.id())), &data)?;
        File::open(&bad)?.sync_all()?;
        Ok(())
    }

    fn release(&self, len: u64) {
        let _ = self
            .bytes
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |b| {
                Some(b.saturating_sub(len))
            });
    }
}

/// A spill written and fsynced under a temporary name. Dropped without
/// [`Prepared::publish`], the file is deleted and its bytes uncounted.
#[derive(Debug)]
pub(crate) struct Prepared<'a> {
    journal: &'a Journal,
    tmp: PathBuf,
    name: String,
    len: u64,
    published: bool,
}

impl Prepared<'_> {
    /// Renames the file to `<ulid>.json` and fsyncs the directory.
    pub(crate) fn publish(mut self) -> Result<(), SpillError> {
        let dir = &self.journal.dir;
        fs::rename(&self.tmp, dir.join(&self.name)).map_err(|e| SpillError::Io(e.into()))?;
        // From here the file is visible and will be committed; keep it.
        self.published = true;
        File::open(dir)
            .and_then(|d| d.sync_all())
            .map_err(|e| SpillError::Io(e.into()))
    }
}

impl Drop for Prepared<'_> {
    fn drop(&mut self) {
        if !self.published {
            let _ = fs::remove_file(&self.tmp);
            self.journal.release(self.len);
        }
    }
}

/// Creates (or replaces) `path` with mode 0600, writes `data` and fsyncs it.
fn write_synced(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let mut o = OpenOptions::new();
    o.write(true).create(true).truncate(true);
    crate::packs::harden_open(&mut o);
    let mut f = o.open(path)?;
    f.write_all(data)?;
    f.sync_all()
}
