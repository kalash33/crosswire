//! Durable exchange store (SQLite plus content packs) for Crosswire.
//!
//! The on-disk format is documented in `docs/format.md`. [`Store`] is the
//! entry point for recording: it runs the writer thread, hands out the
//! [`crosswire_model::ExchangeSink`], the event stream and the audit log.

mod atomic;
mod audit;
mod bodies;
mod control;
mod error;
mod events;
mod journal;
mod lock;
mod packs;
mod project;
mod read;
mod schema;
mod sink;
mod store;
mod sweep;
#[cfg(feature = "synthetic")]
pub mod synthetic;
mod writer;

pub use bodies::{BodyLocator, BodySide, BodyStore};
pub use control::{Control, Draft, TokenRecord, TokenScope};
pub use error::StoreError;
pub use events::EventStream;
pub use packs::{BlobRef, PackWriter};
pub use project::Project;
pub use read::{
    Counts, ExchangeFilter, ExchangeRow, NamespaceCounts, Page, PAGE_LIMIT_MAX, PATH_SCAN_CAP,
};
pub use store::{OpenReport, Store, StoreConfig};
pub use sweep::{sweep, SweepReport};
#[cfg(feature = "synthetic")]
pub use synthetic::generate;

/// The project database format (`PRAGMA user_version`) this build creates and
/// opens; reported by the API health endpoint.
pub const FORMAT_VERSION: u32 = schema::USER_VERSION;
