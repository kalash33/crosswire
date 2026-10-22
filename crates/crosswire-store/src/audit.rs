//! The append-only audit log (spec section 9.4).

use crate::error::StoreError;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use ulid::Ulid;

/// `audit.jsonl` in the project directory, rotated to `audit.jsonl.1`.
#[derive(Debug)]
pub(crate) struct AuditLog {
    dir: PathBuf,
    /// `None` after a rotation whose new file could not be created; the next
    /// append tries again rather than writing into the renamed generation.
    file: Option<File>,
    len: u64,
    rotate_at: u64,
}

impl AuditLog {
    pub(crate) fn open(dir: &Path, rotate_at: u64) -> Result<AuditLog, StoreError> {
        let file = open_log(&dir.join("audit.jsonl"))?;
        let len = file.metadata()?.len();
        Ok(AuditLog {
            dir: dir.to_path_buf(),
            file: Some(file),
            len,
            rotate_at,
        })
    }

    /// Appends one JSON line and `fdatasync`s it before returning.
    pub(crate) fn append(
        &mut self,
        actor: &str,
        action: &str,
        target: &str,
        result: &str,
    ) -> Result<(), StoreError> {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
        let mut line = serde_json::to_string(&serde_json::json!({
            "ts": ts,
            "ulid": Ulid::new().to_string(),
            "actor": actor,
            "action": action,
            "target": target,
            "result": result,
        }))?;
        line.push('\n');
        let add = line.len() as u64;
        if self.len > 0 && self.len + add > self.rotate_at {
            self.rotate()?;
        }
        let file = match &mut self.file {
            Some(f) => f,
            None => self.file.insert(self.new_generation()?),
        };
        file.write_all(line.as_bytes())?;
        file.sync_data()?;
        self.len += add;
        Ok(())
    }

    /// Renames the full log to `audit.jsonl.1` (replacing the previous
    /// generation) and starts an empty one.
    fn rotate(&mut self) -> Result<(), StoreError> {
        let cur = self.dir.join("audit.jsonl");
        if let Some(f) = self.file.take() {
            f.sync_all()?;
            // Closed here: nothing more goes into the old generation.
            drop(f);
        }
        std::fs::rename(&cur, self.dir.join("audit.jsonl.1"))?;
        self.len = 0;
        self.file = Some(self.new_generation()?);
        Ok(())
    }

    /// Creates `audit.jsonl` and makes its directory entry durable.
    fn new_generation(&self) -> Result<File, StoreError> {
        let file = open_log(&self.dir.join("audit.jsonl"))?;
        File::open(&self.dir)?.sync_all()?;
        Ok(file)
    }
}

fn open_log(path: &Path) -> Result<File, StoreError> {
    let mut o = OpenOptions::new();
    o.append(true).create(true);
    crate::packs::harden_open(&mut o);
    Ok(o.open(path)?)
}
