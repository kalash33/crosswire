//! Durable exchange store (SQLite plus content packs) for Crosswire.
//!
//! The on-disk format is documented in `docs/format.md`. [`Store`] is the
//! entry point for recording: it runs the writer thread, hands out the
//! [`crosswire_model::ExchangeSink`], the event stream and the audit log.

mod audit;
mod bodies;
mod error;
mod events;
mod journal;
mod lock;
mod packs;
mod project;
mod schema;
mod sink;
mod store;
mod sweep;
mod writer;

pub use bodies::{BodyLocator, BodySide, BodyStore};
pub use error::StoreError;
pub use events::EventStream;
pub use packs::{BlobRef, PackWriter};
pub use project::Project;
pub use store::{Store, StoreConfig};
pub use sweep::{sweep, SweepReport};
