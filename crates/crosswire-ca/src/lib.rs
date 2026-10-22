//! Local certificate authority and leaf certificate minting for Crosswire.
//!
//! [`Ca`] is a P-256 root persisted through a [`KeyStore`]; [`LeafIssuer`] mints
//! and caches per-host leaves as `rustls` [`CertifiedKey`](rustls::sign::CertifiedKey)s.

mod ca;
mod error;
mod issuer;
mod keystore;

pub use ca::Ca;
pub use error::CaError;
pub use issuer::{IssuerLimits, LeafIssuer};
pub use keystore::{FileKeyStore, KeyStore};
