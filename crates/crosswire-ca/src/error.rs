//! Errors for CA handling and leaf issuance.

/// Errors from the CA, key store and leaf issuer.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CaError {
    /// Key or certificate generation failed.
    #[error("certificate generation failed: {0}")]
    Generate(String),
    /// A PEM or DER input was malformed, or the certificate and key do not match.
    #[error("invalid PEM or certificate: {0}")]
    Pem(String),
    /// Reading or writing the key store failed.
    #[error("key store I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// The leaf mint rate was exceeded; retry shortly.
    #[error("leaf mint rate exceeded")]
    RateLimited,
    /// The server name cannot be certified.
    #[error("host cannot be certified")]
    InvalidHost,
    /// The CA is expired or too close to expiry to sign a valid leaf.
    #[error("CA certificate is expired or about to expire")]
    CaExpired,
}
