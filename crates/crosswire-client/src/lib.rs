//! Recording upstream HTTP/1.1 client for Crosswire.
//!
//! [`Transport`] resolves a [`Target`], passes the resolved addresses through
//! the [`DestinationGate`], connects, and runs TLS through a [`TlsBackend`].
//!
//! [`Session`] sends a [`WritePlan`] over a [`Connection`] and reads the
//! response with `crosswire-h1`, one [`ResponseStream`] per exchange.

mod error;
mod gate;
mod rustls_backend;
mod session;
mod stream;
mod tls;
mod transport;
mod write_plan;

pub use error::ClientError;
pub use gate::{DestinationGate, GateRefusal};
pub use rustls_backend::RustlsBackend;
pub use session::Session;
pub use stream::{BodyPiece, ResponseStream};
pub use tls::{BoxFuture, Conn, Handshake, TlsBackend, TlsConnectConfig, TlsInfo, VerifyMode};
pub use transport::{Connection, Target, Transport};
pub use write_plan::{HoldPoint, Segment, WritePlan};
