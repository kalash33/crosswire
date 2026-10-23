//! A project directory: layout, writer connection, readers.

use crate::error::StoreError;
use crate::lock::ProjectLock;
use crate::schema::{self, APPLICATION_ID, USER_VERSION};
use rusqlite::config::DbConfig;
use rusqlite::{Connection, ErrorCode, OpenFlags};
use std::fs::{self, DirBuilder, OpenOptions};
use std::path::{Path, PathBuf};

pub(crate) const DB_FILE: &str = "project.db";

#[cfg(test)]
thread_local! {
    /// Counts `PRAGMA integrity_check` runs on this thread (tests only).
    pub(crate) static INTEGRITY_CHECKS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// An open project. Owns the one writer connection and the project lock.
///
/// `Project` is `Send` but not `Sync`: the store moves it into the writer
/// thread, which is then the only user of the writer connection.
#[derive(Debug)]
pub struct Project {
    dir: PathBuf,
    db: Connection,
    lock: ProjectLock,
}

impl Project {
    /// Creates a new project in `dir` (made with mode 0700 on unix if absent).
    ///
    /// Fails with [`StoreError::AlreadyExists`] if `project.db` exists, before
    /// taking the lock, and with [`StoreError::Locked`] if the directory is
    /// held. An existing directory's permissions are never changed.
    pub fn create(dir: &Path) -> Result<Project, StoreError> {
        if dir.join(DB_FILE).exists() {
            return Err(StoreError::AlreadyExists);
        }
        make_dir(dir)?;
        let lock = ProjectLock::acquire(&dir.join("project.lock"))?;
        Project::create_locked(dir, lock)
    }

    fn create_locked(dir: &Path, lock: ProjectLock) -> Result<Project, StoreError> {
        let db_path = dir.join(DB_FILE);
        let mut create = OpenOptions::new();
        create.write(true).create_new(true);
        restrict(&mut create);
        create.open(&db_path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                StoreError::AlreadyExists
            } else {
                StoreError::Io(e)
            }
        })?;
        make_real_dir(&dir.join("packs"), "packs/")?;
        make_real_dir(&dir.join("journal"), "journal/")?;
        let mut audit = OpenOptions::new();
        audit.append(true).create(true);
        restrict(&mut audit);
        audit.open(dir.join("audit.jsonl"))?;

        let mut db = open_writer(&db_path)?;
        schema::init(&mut db)?;
        configure_writer(&db)?;
        Ok(Project {
            dir: dir.to_path_buf(),
            db,
            lock,
        })
    }

    /// Opens an existing project.
    ///
    /// Refuses a database whose `application_id` is not Crosswire's, or whose
    /// `user_version` is 0 ([`StoreError::NotAProject`]) or above 1
    /// ([`StoreError::NewerFormat`]), and (before taking the lock) a project
    /// whose `project.db`, `-wal` or `-shm` is a symlink ([`StoreError::Corrupt`]). Runs `PRAGMA integrity_check` only when
    /// the previous run did not shut down cleanly.
    pub fn open(dir: &Path) -> Result<Project, StoreError> {
        refuse_symlinked_db(dir)?;
        if !dir.join(DB_FILE).is_file() {
            return Err(StoreError::NotAProject);
        }
        let lock = ProjectLock::acquire(&dir.join("project.lock"))?;
        Project::open_locked(dir, lock)
    }

    fn open_locked(dir: &Path, lock: ProjectLock) -> Result<Project, StoreError> {
        let db = open_writer(&dir.join(DB_FILE)).map_err(classify)?;
        let app: i64 = db
            .query_row("PRAGMA application_id", [], |r| r.get(0))
            .map_err(classify_sqlite)?;
        if app != APPLICATION_ID {
            return Err(StoreError::NotAProject);
        }
        let version: u32 = db
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(classify_sqlite)?;
        if version == 0 {
            return Err(StoreError::NotAProject);
        }
        if version > USER_VERSION {
            return Err(StoreError::NewerFormat(version));
        }
        if lock.previous_unclean() {
            #[cfg(test)]
            INTEGRITY_CHECKS.with(|c| c.set(c.get() + 1));
            let result: String = db
                .query_row("PRAGMA integrity_check", [], |r| r.get(0))
                .map_err(classify_sqlite)?;
            if result != "ok" {
                return Err(StoreError::Corrupt(result));
            }
        }
        configure_writer(&db).map_err(classify)?;
        Ok(Project {
            dir: dir.to_path_buf(),
            db,
            lock,
        })
    }

    /// Opens the project in `dir`, creating it if there is none.
    ///
    /// A 0-byte `project.db` (a crash between creating the file and writing
    /// the schema) is deleted and recreated.
    pub fn open_or_create(dir: &Path) -> Result<Project, StoreError> {
        make_dir(dir)?;
        refuse_symlinked_db(dir)?;
        let lock = ProjectLock::acquire(&dir.join("project.lock"))?;
        let db_path = dir.join(DB_FILE);
        match fs::metadata(&db_path) {
            Ok(m) if m.len() == 0 => {
                fs::remove_file(&db_path)?;
                for ext in ["-wal", "-shm"] {
                    let _ = fs::remove_file(dir.join(format!("{DB_FILE}{ext}")));
                }
                Project::create_locked(dir, lock)
            }
            Ok(_) => Project::open_locked(dir, lock),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Project::create_locked(dir, lock),
            Err(e) => Err(e.into()),
        }
    }

    /// The project directory.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// A new read-only connection with bounded cache and mmap.
    pub fn reader(&self) -> Result<Connection, StoreError> {
        open_reader(&self.dir)
    }

    /// Records a clean shutdown in the lock file (the store's `close` calls this).
    pub fn mark_clean(&self) -> Result<(), StoreError> {
        self.lock.mark_clean()
    }

    /// Records `clean=0` in the lock file.
    pub(crate) fn mark_dirty(&self) -> Result<(), StoreError> {
        self.lock.mark_dirty()
    }

    /// Whether the run before this one left `clean=0` (it did not close).
    pub(crate) fn previous_unclean(&self) -> bool {
        self.lock.previous_unclean()
    }

    /// The writer connection; the writer thread uses it to run transactions.
    pub fn writer_mut(&mut self) -> &mut Connection {
        &mut self.db
    }

    /// The path of pack `id`, derived from the id and never read from the database.
    #[must_use]
    pub fn pack_path(&self, id: u32) -> PathBuf {
        crate::packs::pack_file(&self.dir, id)
    }

    /// Records that the first `len` bytes of pack `id` are committed.
    ///
    /// Call inside the transaction that commits the batch whose records end at
    /// `len`, after the pack was fsynced. Never lowers the stored value; fails if the pack has no row.
    pub fn set_pack_committed_len(
        tx: &rusqlite::Transaction<'_>,
        id: u32,
        len: u64,
    ) -> Result<(), StoreError> {
        let len = i64::try_from(len).map_err(|_| StoreError::Corrupt("pack too large".into()))?;
        let n = tx.execute(
            "UPDATE packs SET committed_len = max(committed_len, ?2) WHERE id = ?1",
            rusqlite::params![id, len],
        )?;
        if n == 0 {
            return Err(StoreError::Corrupt(format!("no packs row for pack {id}")));
        }
        Ok(())
    }
}

/// A read-only connection to the project in `dir` (see [`Project::reader`]).
pub(crate) fn open_reader(dir: &Path) -> Result<Connection, StoreError> {
    let conn = Connection::open_with_flags(
        dir.join(DB_FILE),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    harden(&conn)?;
    conn.execute_batch(
        "PRAGMA query_only=ON; PRAGMA busy_timeout=5000; \
         PRAGMA cache_size=-16384; PRAGMA mmap_size=268435456;",
    )?;
    Ok(conn)
}

/// Maps `SQLITE_CORRUPT` and `SQLITE_NOTADB` to their store errors.
fn classify_sqlite(e: rusqlite::Error) -> StoreError {
    classify(StoreError::Sqlite(e))
}

fn classify(e: StoreError) -> StoreError {
    if let StoreError::Sqlite(rusqlite::Error::SqliteFailure(f, _)) = &e {
        match f.code {
            ErrorCode::DatabaseCorrupt => return StoreError::Corrupt(e.to_string()),
            ErrorCode::NotADatabase => return StoreError::NotAProject,
            _ => {}
        }
    }
    e
}

fn open_writer(path: &Path) -> Result<Connection, StoreError> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    harden(&conn)?;
    Ok(conn)
}

/// Defensive mode and no trusted schema: a hostile project file must not run code.
fn harden(conn: &Connection) -> Result<(), StoreError> {
    conn.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)?;
    conn.set_db_config(DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA, false)?;
    conn.pragma_update(None, "trusted_schema", "OFF")?;
    Ok(())
}

fn configure_writer(conn: &Connection) -> Result<(), StoreError> {
    let mode: String = conn.pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get(0))?;
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(StoreError::Corrupt(format!("journal_mode is {mode}")));
    }
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    // On macOS plain fsync does not flush the drive cache; make SQLite's own
    // checkpoint syncs use F_FULLFSYNC (a no-op elsewhere).
    conn.pragma_update(None, "checkpoint_fullfsync", "ON")?;
    conn.pragma_update(None, "busy_timeout", 5000)?;
    Ok(())
}

/// Creates `path` (and missing parents) with mode 0700; existing directories are left alone.
pub(crate) fn make_dir(path: &Path) -> Result<(), StoreError> {
    let mut b = DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        b.mode(0o700);
    }
    b.create(path)?;
    Ok(())
}

/// Like [`make_dir`], then requires `path` itself to be a real directory, not
/// a symlink: a hostile project must not redirect our moves and deletes.
pub(crate) fn make_real_dir(path: &Path, label: &str) -> Result<(), StoreError> {
    make_dir(path)?;
    if fs::symlink_metadata(path)?.file_type().is_dir() {
        Ok(())
    } else {
        Err(StoreError::Corrupt(format!(
            "{label} is a symlink or not a directory"
        )))
    }
}

/// Refuses a project whose database files are symlinks. `SQLite` writes and
/// truncates the `-wal` and `-shm` files, so it must never follow one out of
/// the project.
fn refuse_symlinked_db(dir: &Path) -> Result<(), StoreError> {
    for name in [DB_FILE, "project.db-wal", "project.db-shm"] {
        match fs::symlink_metadata(dir.join(name)) {
            Ok(m) if m.file_type().is_symlink() => {
                return Err(StoreError::Corrupt(format!("{name} is a symlink")));
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

fn restrict(opts: &mut OpenOptions) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    #[cfg(not(unix))]
    let _ = opts;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count() -> u32 {
        INTEGRITY_CHECKS.with(std::cell::Cell::get)
    }

    #[test]
    fn clean_previous_run_skips_integrity_check_unclean_runs_it() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("p");
        let p = Project::create(&dir).unwrap();
        p.mark_clean().unwrap();
        drop(p);
        let before = count();
        let p = Project::open(&dir).unwrap();
        assert_eq!(count(), before, "clean previous run must skip the check");
        drop(p); // leaves clean=0
        drop(Project::open(&dir).unwrap());
        assert_eq!(count(), before + 1, "unclean previous run must check");
    }

    #[test]
    fn writer_mut_starts_transactions() {
        let tmp = tempfile::tempdir().unwrap();
        let mut p = Project::create(&tmp.path().join("p")).unwrap();
        let tx = p.writer_mut().transaction().unwrap();
        tx.execute("INSERT INTO meta(key, value) VALUES ('k', 'v')", [])
            .unwrap();
        tx.commit().unwrap();
        assert_eq!(p.pack_path(7).file_name().unwrap(), "000007.pack");
        let full: i64 = p
            .writer_mut()
            .query_row("PRAGMA checkpoint_fullfsync", [], |r| r.get(0))
            .unwrap();
        assert_eq!(full, 1);
    }

    #[test]
    fn project_is_send() {
        fn send<T: Send>() {}
        send::<Project>();
    }
}
