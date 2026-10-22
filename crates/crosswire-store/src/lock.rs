//! The project lock: an exclusive `flock` plus a small status file.

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
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut file = opts.open(path)?;
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
        let mut text = String::new();
        let mut f = &self.file;
        f.seek(SeekFrom::Start(0))?;
        f.read_to_string(&mut text)?;
        let head: Vec<&str> = text.lines().take(2).collect();
        let status = format!("{}\nclean=1\n", head.join("\n"));
        self.write_status(&status)
    }

    fn write_status(&self, text: &str) -> Result<(), StoreError> {
        let mut f = &self.file;
        self.file.set_len(0)?;
        f.seek(SeekFrom::Start(0))?;
        f.write_all(text.as_bytes())?;
        self.file.sync_data()?;
        Ok(())
    }
}
