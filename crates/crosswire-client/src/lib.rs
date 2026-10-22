//! Recording upstream HTTP/1.1 client for Crosswire.
//!
//! [`Transport`] resolves a [`Target`], passes the resolved addresses through
//! the [`DestinationGate`], connects, and runs TLS through a [`TlsBackend`].

mod error;
mod gate;
mod rustls_backend;
mod tls;
mod transport;

pub use error::ClientError;
pub use gate::{DestinationGate, GateRefusal};
pub use rustls_backend::RustlsBackend;
pub use tls::{BoxFuture, Conn, Handshake, TlsBackend, TlsConnectConfig, TlsInfo, VerifyMode};
pub use transport::{Connection, Target, Transport};
