//! A project directory: layout, writer connection, readers.

use crate::error::StoreError;
use crate::lock::ProjectLock;
use crate::schema::{self, APPLICATION_ID, USER_VERSION};
use rusqlite::config::DbConfig;
use rusqlite::{Connection, OpenFlags};
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};

const DB_FILE: &str = "project.db";

/// An open project. Owns the one writer connection and the project lock.
#[derive(Debug)]
pub struct Project {
    dir: PathBuf,
    db: Connection,
    lock: ProjectLock,
}

impl Project {
    /// Creates a new project in `dir` (made with mode 0700 on unix).
    ///
    /// Fails with [`StoreError::Locked`] if the project is held, and with an
    /// I/O error if `project.db` already exists.
    pub fn create(dir: &Path) -> Result<Project, StoreError> {
        make_dir(dir)?;
        let lock = ProjectLock::acquire(&dir.join("project.lock"))?;
        let db_path = dir.join(DB_FILE);
        let mut create = OpenOptions::new();
        create.write(true).create_new(true);
        restrict(&mut create);
        create.open(&db_path)?;
        make_dir(&dir.join("packs"))?;
        make_dir(&dir.join("journal"))?;
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
    /// ([`StoreError::NewerFormat`]). Runs `PRAGMA integrity_check` only when
    /// the previous run did not shut down cleanly.
    pub fn open(dir: &Path) -> Result<Project, StoreError> {
        let db_path = dir.join(DB_FILE);
        if !db_path.is_file() {
            return Err(StoreError::NotAProject);
        }
        let lock = ProjectLock::acquire(&dir.join("project.lock"))?;
        let db = open_writer(&db_path)?;
        let app: i64 = db.query_row("PRAGMA application_id", [], |r| r.get(0))?;
        if app != APPLICATION_ID {
            return Err(StoreError::NotAProject);
        }
        let version: u32 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version == 0 {
            return Err(StoreError::NotAProject);
        }
        if version > USER_VERSION {
            return Err(StoreError::NewerFormat(version));
        }
        if lock.previous_unclean() {
            let result: String = db.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
            if result != "ok" {
                return Err(StoreError::Corrupt(result));
            }
        }
        configure_writer(&db)?;
        Ok(Project {
            dir: dir.to_path_buf(),
            db,
            lock,
        })
    }

    /// Opens the project in `dir`, creating it if `project.db` does not exist.
    pub fn open_or_create(dir: &Path) -> Result<Project, StoreError> {
        if dir.join(DB_FILE).exists() {
            Project::open(dir)
        } else {
            Project::create(dir)
        }
    }

    /// The project directory.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// A new read-only connection with bounded cache and mmap.
    pub fn reader(&self) -> Result<Connection, StoreError> {
        let conn = Connection::open_with_flags(
            self.dir.join(DB_FILE),
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        harden(&conn)?;
        conn.execute_batch(
            "PRAGMA query_only=ON; PRAGMA busy_timeout=5000; \
             PRAGMA cache_size=-16384; PRAGMA mmap_size=268435456;",
        )?;
        Ok(conn)
    }

    /// Records a clean shutdown in the lock file (the store's `close` calls this).
    pub fn mark_clean(&self) -> Result<(), StoreError> {
        self.lock.mark_clean()
    }

    /// The writer connection, for the writer thread.
    #[allow(dead_code)]
    pub(crate) fn writer(&self) -> &Connection {
        &self.db
    }
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
    conn.pragma_update(None, "busy_timeout", 5000)?;
    Ok(())
}

fn make_dir(path: &Path) -> Result<(), StoreError> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
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
