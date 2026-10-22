//! Durable exchange store (SQLite plus content packs) for Crosswire.
//!
//! The on-disk format is documented in `docs/format.md`.

mod error;
mod lock;
mod project;
mod schema;

pub use error::StoreError;
pub use project::Project;
