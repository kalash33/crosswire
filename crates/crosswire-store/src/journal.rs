//! The spill journal: one fsynced JSON file per exchange the queue had no room for.
//!
//! A spill is written to `journal/<ulid>.json.tmp`, fsynced, renamed to
//! `journal/<ulid>.json` and the directory fsynced, so a `.json` file is
//! always complete. Leftover `.tmp` files were never acknowledged and are
//! deleted on open. Unreadable files are moved to `journal/bad/`.

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
        crate::project::make_dir(&dir)?;
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

    /// Writes `ex` durably. `before_publish` runs after the data is fsynced
    /// and before the file becomes visible to the writer under its final name.
    pub(crate) fn spill(
        &self,
        ex: &Exchange,
        before_publish: impl FnOnce(),
    ) -> Result<(), SpillError> {
        let data = serde_json::to_vec(ex).map_err(|e| SpillError::Io(e.into()))?;
        let len = data.len() as u64;
        if self
            .bytes
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |b| {
                b.checked_add(len).filter(|n| *n <= self.cap)
            })
            .is_err()
        {
            return Err(SpillError::CapExceeded);
        }
        let name = format!("{}.json", ex.id());
        let tmp = self.dir.join(format!("{name}.tmp"));
        let written = write_synced(&tmp, &data)
            .map(|()| before_publish())
            .and_then(|()| fs::rename(&tmp, self.dir.join(&name)))
            .and_then(|()| File::open(&self.dir)?.sync_all());
        if let Err(e) = written {
            let _ = fs::remove_file(&tmp);
            self.release(len);
            return Err(SpillError::Io(e.into()));
        }
        Ok(())
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
    pub(crate) fn read(entry: &Entry) -> Result<Exchange, String> {
        let mut o = OpenOptions::new();
        o.read(true);
        crate::packs::harden_open(&mut o);
        let mut f = o.open(&entry.path).map_err(|e| e.to_string())?;
        let meta = f.metadata().map_err(|e| e.to_string())?;
        if !meta.is_file() {
            return Err("not a regular file".into());
        }
        if meta.len() > MAX_FILE {
            return Err("file too large".into());
        }
        let mut data = Vec::new();
        Read::by_ref(&mut f)
            .take(MAX_FILE + 1)
            .read_to_end(&mut data)
            .map_err(|e| e.to_string())?;
        serde_json::from_slice(&data).map_err(|e| e.to_string())
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
        crate::project::make_dir(&bad)?;
        let name = entry.path.file_name().unwrap_or_default();
        fs::rename(&entry.path, bad.join(name))?;
        self.release(entry.len);
        Ok(())
    }

    /// Writes an exchange that could not be committed to `journal/bad/`.
    pub(crate) fn quarantine_exchange(&self, ex: &Exchange) -> Result<(), StoreError> {
        let bad = self.dir.join("bad");
        crate::project::make_dir(&bad)?;
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

/// Creates (or replaces) `path` with mode 0600, writes `data` and fsyncs it.
fn write_synced(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let mut o = OpenOptions::new();
    o.write(true).create(true).truncate(true);
    crate::packs::harden_open(&mut o);
    let mut f = o.open(path)?;
    f.write_all(data)?;
    f.sync_all()
}
