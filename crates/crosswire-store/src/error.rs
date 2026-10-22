//! Store errors.

/// What can go wrong opening or using a project.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StoreError {
    /// A filesystem operation failed.
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    /// `SQLite` reported an error.
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// Another process (or another `Project` in this one) holds the project.
    #[error("project is locked by another instance")]
    Locked,
    /// The project was written by a newer format version than this build knows.
    #[error("project format version {0} is newer than this build supports")]
    NewerFormat(u32),
    /// The directory is not a Crosswire project.
    #[error("not a crosswire project")]
    NotAProject,
    /// The project failed an integrity check.
    #[error("project is corrupt: {0}")]
    Corrupt(String),
    /// The writer queue is full.
    #[error("writer queue is full")]
    QueueFull,
    /// The store is closed.
    #[error("store is closed")]
    Closed,
}
