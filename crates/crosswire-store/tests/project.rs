//! Project layout, schema, locking and open-time checks.

use crosswire_store::{Project, StoreError};
use rusqlite::Connection;

fn raw(dir: &std::path::Path) -> Connection {
    Connection::open(dir.join("project.db")).unwrap()
}

#[test]
fn create_then_open_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("proj");
    let p = Project::create(&dir).unwrap();
    assert_eq!(p.dir(), dir);
    drop(p);
    for sub in ["packs", "journal"] {
        assert!(dir.join(sub).is_dir(), "{sub}");
    }
    for f in ["project.db", "audit.jsonl", "project.lock"] {
        assert!(dir.join(f).is_file(), "{f}");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode =
            |p: std::path::PathBuf| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(dir.clone()), 0o700);
        assert_eq!(mode(dir.join("project.db")), 0o600);
        assert_eq!(mode(dir.join("audit.jsonl")), 0o600);
        assert_eq!(mode(dir.join("project.lock")), 0o600);
    }
    let p = Project::open(&dir).unwrap();
    let r = p.reader().unwrap();
    let v: i64 = r
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(v, 6);
    let a: i64 = r
        .query_row("PRAGMA application_id", [], |r| r.get(0))
        .unwrap();
    assert_eq!(a, 0x4357_5052);
    let n: i64 = r
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name IN \
             ('exchanges','bodies','blobs','packs','markers','meta')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 6);
    let jm: String = r
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .unwrap();
    assert_eq!(jm, "wal");
}

#[test]
fn fresh_project_is_format_6() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("p");
    drop(Project::create(&dir).unwrap());
    let v: i64 = raw(&dir)
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(v, 6);
}

#[test]
fn open_refuses_pre_release_project() {
    for version in [5_u32, 1] {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("p");
        drop(Project::create(&dir).unwrap());
        raw(&dir)
            .pragma_update(None, "user_version", version)
            .unwrap();
        let err = Project::open(&dir).unwrap_err();
        assert!(
            matches!(err, StoreError::PreReleaseProject { version: v } if v == version),
            "version {version}: {err:?}"
        );
        assert_eq!(
            err.to_string(),
            format!(
                "project was created by a pre-release build of Crosswire (format {version}); \
                 re-record it with 0.1.0"
            )
        );
        // Refused before any migration or write: no snapshot, version untouched.
        assert!(!dir.join(format!("project.db.v{version}.bak")).exists());
        let v: u32 = raw(&dir)
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, version);
    }
}

#[test]
fn open_or_create_does_both() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("p");
    drop(Project::open_or_create(&dir).unwrap());
    drop(Project::open_or_create(&dir).unwrap());
}

#[test]
fn second_open_is_locked() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("p");
    let first = Project::create(&dir).unwrap();
    assert!(matches!(Project::open(&dir), Err(StoreError::Locked)));
    assert!(matches!(
        Project::open_or_create(&dir),
        Err(StoreError::Locked)
    ));
    drop(first);
    drop(Project::open(&dir).unwrap());
}

#[test]
fn open_still_rejects_newer_project() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("p");
    drop(Project::create(&dir).unwrap());
    raw(&dir).pragma_update(None, "user_version", 7).unwrap();
    assert!(matches!(
        Project::open(&dir),
        Err(StoreError::NewerFormat(7))
    ));
}

#[test]
fn wrong_application_id_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("p");
    drop(Project::create(&dir).unwrap());
    raw(&dir).pragma_update(None, "application_id", 7).unwrap();
    assert!(matches!(Project::open(&dir), Err(StoreError::NotAProject)));
}

#[test]
fn uninitialised_db_is_not_a_project() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("p");
    drop(Project::create(&dir).unwrap());
    raw(&dir).pragma_update(None, "user_version", 0).unwrap();
    assert!(matches!(Project::open(&dir), Err(StoreError::NotAProject)));
}

#[test]
fn missing_db_is_not_a_project() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(matches!(
        Project::open(tmp.path()),
        Err(StoreError::NotAProject)
    ));
}

#[test]
fn reader_is_read_only() {
    let tmp = tempfile::tempdir().unwrap();
    let p = Project::create(&tmp.path().join("p")).unwrap();
    let r = p.reader().unwrap();
    let err = r
        .execute("INSERT INTO meta(key, value) VALUES ('x', 'y')", [])
        .unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("readonly")
            || err.to_string().contains("read-only"),
        "{err}"
    );
    let ts: i64 = r
        .query_row("PRAGMA trusted_schema", [], |r| r.get(0))
        .unwrap();
    assert_eq!(ts, 0);
}

#[test]
fn lock_file_tracks_clean_shutdown() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("p");
    let p = Project::create(&dir).unwrap();
    let text = std::fs::read_to_string(dir.join("project.lock")).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 3, "{text}");
    assert_eq!(lines[0], std::process::id().to_string());
    assert!(lines[1].parse::<u64>().is_ok());
    assert_eq!(lines[2], "clean=0");
    p.mark_clean().unwrap();
    let text = std::fs::read_to_string(dir.join("project.lock")).unwrap();
    assert!(text.ends_with("clean=1\n"), "{text}");
}

#[test]
fn unclean_previous_run_triggers_integrity_check_and_corruption_is_reported() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("p");
    // Clean cycle: opens fine.
    let p = Project::create(&dir).unwrap();
    p.mark_clean().unwrap();
    drop(p);
    drop(Project::open(&dir).unwrap()); // leaves clean=0 behind (simulated crash)
                                        // Previous run was unclean: still opens (db is healthy).
    drop(Project::open(&dir).unwrap());
    // Now damage the file after an unclean run: pages beyond the header.
    let db = dir.join("project.db");
    let mut bytes = std::fs::read(&db).unwrap();
    for b in bytes.iter_mut().skip(4096).take(8192) {
        *b = 0xA5;
    }
    std::fs::write(&db, bytes).unwrap();
    let _ = std::fs::remove_file(dir.join("project.db-wal"));
    let _ = std::fs::remove_file(dir.join("project.db-shm"));
    assert!(matches!(Project::open(&dir), Err(StoreError::Corrupt(_))));
}

#[test]
fn create_refuses_existing_project_before_locking() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("p");
    let first = Project::create(&dir).unwrap();
    // Held: AlreadyExists wins over Locked, because it is checked first.
    assert!(matches!(
        Project::create(&dir),
        Err(StoreError::AlreadyExists)
    ));
    drop(first);
    assert!(matches!(
        Project::create(&dir),
        Err(StoreError::AlreadyExists)
    ));
}

#[cfg(unix)]
#[test]
fn create_does_not_chmod_an_existing_directory() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("shared");
    std::fs::create_dir(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    drop(Project::create(&dir).unwrap());
    let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&dir), 0o755);
    assert_eq!(mode(&dir.join("packs")), 0o700);
    assert_eq!(mode(&dir.join("journal")), 0o700);
}

#[test]
fn open_or_create_recreates_a_zero_byte_db() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("p");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("project.db"), b"").unwrap();
    let p = Project::open_or_create(&dir).unwrap();
    let v: i64 = p
        .reader()
        .unwrap()
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(v, 6);
}

#[test]
fn garbage_file_is_not_a_project() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("p");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("project.db"), vec![0x41u8; 8192]).unwrap();
    assert!(matches!(Project::open(&dir), Err(StoreError::NotAProject)));
}

#[test]
fn packs_table_has_no_path_column() {
    let tmp = tempfile::tempdir().unwrap();
    let p = Project::create(&tmp.path().join("p")).unwrap();
    let cols: Vec<String> = p
        .reader()
        .unwrap()
        .prepare("SELECT name FROM pragma_table_info('packs')")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(cols, ["id", "committed_len"]);
}

#[cfg(unix)]
#[test]
fn symlinked_lock_file_is_refused_and_target_untouched() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("p");
    drop(Project::create(&dir).unwrap());
    let outside = tmp.path().join("authorized_keys");
    let original = b"ssh-ed25519 AAAA... user@host\n".repeat(4);
    std::fs::write(&outside, &original).unwrap();
    std::fs::remove_file(dir.join("project.lock")).unwrap();
    std::os::unix::fs::symlink(&outside, dir.join("project.lock")).unwrap();
    assert!(Project::open(&dir).is_err());
    assert!(Project::open_or_create(&dir).is_err());
    assert_eq!(std::fs::read(&outside).unwrap(), original);
}

#[cfg(unix)]
#[test]
fn symlinked_database_files_are_refused() {
    for name in ["project.db", "project.db-wal", "project.db-shm"] {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("p");
        drop(Project::create(&dir).unwrap());
        let real = tmp.path().join("elsewhere");
        if name == "project.db" {
            std::fs::rename(dir.join(name), &real).unwrap();
        } else {
            let _ = std::fs::remove_file(dir.join(name));
            std::fs::write(&real, b"").unwrap();
        }
        std::os::unix::fs::symlink(&real, dir.join(name)).unwrap();
        let err = Project::open(&dir).unwrap_err();
        assert!(
            matches!(&err, StoreError::Corrupt(m) if m.contains("symlink")),
            "{name}: {err:?}"
        );
        let err = Project::open_or_create(&dir).unwrap_err();
        assert!(
            matches!(&err, StoreError::Corrupt(m) if m.contains("symlink")),
            "{name}: {err:?}"
        );
    }
}

#[cfg(unix)]
#[test]
fn symlinked_journal_directory_is_refused() {
    use crosswire_store::{Store, StoreConfig};
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("p");
    drop(Project::create(&dir).unwrap());
    let outside = tmp.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(outside.join("keep.json.tmp"), b"x").unwrap();
    std::fs::write(outside.join("01ARZ3NDEKTSV4RRFFQ69G5FAV.json"), b"not json").unwrap();
    std::fs::remove_dir(dir.join("journal")).unwrap();
    std::os::unix::fs::symlink(&outside, dir.join("journal")).unwrap();
    let err = Store::open(Project::open(&dir).unwrap(), StoreConfig::default()).unwrap_err();
    assert!(
        matches!(&err, StoreError::Corrupt(m) if m.contains("journal")),
        "{err:?}"
    );
    assert!(outside.join("keep.json.tmp").exists());
    assert!(outside.join("01ARZ3NDEKTSV4RRFFQ69G5FAV.json").exists());
    assert!(!outside.join("bad").exists());

    // A symlinked journal/bad is refused too, and nothing is moved into it.
    std::fs::remove_file(dir.join("journal")).unwrap();
    std::fs::create_dir(dir.join("journal")).unwrap();
    std::fs::write(
        dir.join("journal/01ARZ3NDEKTSV4RRFFQ69G5FAV.json"),
        b"not json",
    )
    .unwrap();
    let outside_bad = tmp.path().join("outside_bad");
    std::fs::create_dir(&outside_bad).unwrap();
    std::os::unix::fs::symlink(&outside_bad, dir.join("journal/bad")).unwrap();
    let _ = Store::open(Project::open(&dir).unwrap(), StoreConfig::default());
    assert_eq!(std::fs::read_dir(&outside_bad).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn create_refuses_symlinked_wal_and_audit_log() {
    for name in ["project.db-wal", "audit.jsonl"] {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("p");
        std::fs::create_dir(&dir).unwrap();
        // A dangling symlink: following it would create a file outside.
        let outside = tmp.path().join("outside");
        std::os::unix::fs::symlink(&outside, dir.join(name)).unwrap();
        let err = Project::create(&dir).unwrap_err();
        assert!(
            matches!(&err, StoreError::Corrupt(m) if m.contains("symlink")),
            "{name}: {err:?}"
        );
        assert!(!outside.exists(), "{name}: nothing created outside");
        // An existing target is left byte-identical.
        std::fs::write(&outside, b"keep").unwrap();
        assert!(Project::create(&dir).is_err());
        assert!(Project::open_or_create(&dir).is_err());
        assert_eq!(std::fs::read(&outside).unwrap(), b"keep");
    }
}
