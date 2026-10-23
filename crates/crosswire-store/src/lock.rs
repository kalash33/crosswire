//! The project lock: an exclusive `flock` plus a small status file.

#[cfg(not(unix))]
compile_error!("crosswire-store: the project lock uses flock; Windows support is a later task");

use crate::error::StoreError;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// An exclusive lock on `project.lock`, held until dropped.
///
/// The file holds `pid\nstart_unix_secs\nclean=0|1`. A fresh lock writes
/// `clean=0`; [`ProjectLock::mark_clean`] writes `clean=1`. A previous run that
/// left `clean=0` died without closing.
#[derive(Debug)]
pub(crate) struct ProjectLock {
    file: File,
    previous_unclean: bool,
}

impl ProjectLock {
    /// Takes the lock, reads the previous status and records this run.
    pub(crate) fn acquire(path: &Path) -> Result<ProjectLock, StoreError> {
        let mut opts = OpenOptions::new();
        opts.read(true).write(true).create(true).truncate(false);
        // Mode 0600 and O_NOFOLLOW: a hostile project's `project.lock` symlink
        // must not make us write (and truncate) a file outside the project.
        crate::packs::harden_open(&mut opts);
        let mut file = opts.open(path).map_err(|e| {
            if e.raw_os_error() == Some(rustix::io::Errno::LOOP.raw_os_error()) {
                StoreError::Corrupt("project.lock is a symlink".into())
            } else {
                StoreError::Io(e)
            }
        })?;
        match rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive) {
            Ok(()) => {}
            Err(e) if e == rustix::io::Errno::WOULDBLOCK => return Err(StoreError::Locked),
            Err(e) => return Err(StoreError::Io(e.into())),
        }
        let mut previous = String::new();
        file.read_to_string(&mut previous)?;
        let previous_unclean = previous.lines().any(|l| l == "clean=0");
        let lock = ProjectLock {
            file,
            previous_unclean,
        };
        let started = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        lock.write_status(&format!("{}\n{started}\nclean=0\n", std::process::id()))?;
        Ok(lock)
    }

    /// Whether the previous run recorded `clean=0`.
    pub(crate) fn previous_unclean(&self) -> bool {
        self.previous_unclean
    }

    /// Records a clean shutdown.
    pub(crate) fn mark_clean(&self) -> Result<(), StoreError> {
        self.set_clean(true)
    }

    /// Records that the project is in use (a crash from here on is unclean).
    pub(crate) fn mark_dirty(&self) -> Result<(), StoreError> {
        self.set_clean(false)
    }

    fn set_clean(&self, clean: bool) -> Result<(), StoreError> {
        let mut text = String::new();
        let mut f = &self.file;
        f.seek(SeekFrom::Start(0))?;
        f.read_to_string(&mut text)?;
        let head: Vec<&str> = text.lines().take(2).collect();
        let status = format!("{}\nclean={}\n", head.join("\n"), u8::from(clean));
        self.write_status(&status)
    }

    fn write_status(&self, text: &str) -> Result<(), StoreError> {
        let mut f = &self.file;
        // Write first, then trim: a crash never leaves an empty status file.
        f.seek(SeekFrom::Start(0))?;
        f.write_all(text.as_bytes())?;
        self.file.set_len(text.len() as u64)?;
        self.file.sync_data()?;
        Ok(())
    }
}
