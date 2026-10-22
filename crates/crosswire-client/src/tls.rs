//! The TLS backend trait and its configuration types.

use std::future::Future;
use std::pin::Pin;

use crosswire_model::ChainResult;
use rustls_pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;

use crate::error::ClientError;

/// An established byte stream, plain or TLS.
pub trait Conn: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Conn for T {}

/// How the upstream certificate chain is verified.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum VerifyMode {
    /// Verify against the trust roots and the SNI name; fail the handshake
    /// when the chain is bad.
    WebPki,
    /// Verify as `WebPki` does, but on failure record why and continue. Only
    /// for per-host overrides; never a global default.
    AcceptInvalid,
}

/// Parameters for one TLS client handshake.
pub struct TlsConnectConfig {
    pub(crate) sni: Option<ServerName<'static>>,
    pub(crate) alpn: Vec<Vec<u8>>,
    pub(crate) verify: VerifyMode,
    pub(crate) client_cert: Option<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>)>,
}

impl TlsConnectConfig {
    /// A config with the given verification mode, no SNI, ALPN `http/1.1`
    /// and no client certificate.
    #[must_use]
    pub fn new(verify: VerifyMode) -> TlsConnectConfig {
        TlsConnectConfig {
            sni: None,
            alpn: vec![b"http/1.1".to_vec()],
            verify,
            client_cert: None,
        }
    }

    /// Sets the server name used for SNI and certificate verification.
    #[must_use]
    pub fn with_sni(mut self, sni: ServerName<'static>) -> Self {
        self.sni = Some(sni);
        self
    }

    /// Sets the ALPN protocols offered, in preference order.
    #[must_use]
    pub fn with_alpn(mut self, alpn: Vec<Vec<u8>>) -> Self {
        self.alpn = alpn;
        self
    }

    /// Sets a client certificate chain and key for mutual TLS.
    #[must_use]
    pub fn with_client_cert(
        mut self,
        chain: Vec<CertificateDer<'static>>,
        key: PrivateKeyDer<'static>,
    ) -> Self {
        self.client_cert = Some((chain, key));
        self
    }

    /// The verification mode.
    #[must_use]
    pub fn verify(&self) -> VerifyMode {
        self.verify
    }

    /// The server name, if set.
    #[must_use]
    pub fn sni(&self) -> Option<&ServerName<'static>> {
        self.sni.as_ref()
    }
}

impl Clone for TlsConnectConfig {
    fn clone(&self) -> Self {
        TlsConnectConfig {
            sni: self.sni.clone(),
            alpn: self.alpn.clone(),
            verify: self.verify,
            client_cert: self
                .client_cert
                .as_ref()
                .map(|(c, k)| (c.clone(), k.clone_key())),
        }
    }
}

impl std::fmt::Debug for TlsConnectConfig {
    // The client key is never printed.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TlsConnectConfig")
            .field("sni", &self.sni)
            .field("alpn", &self.alpn.len())
            .field("verify", &self.verify)
            .field("client_cert", &self.client_cert.is_some())
            .finish()
    }
}

/// What the handshake negotiated and how the chain fared.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TlsInfo {
    pub(crate) version: String,
    pub(crate) cipher: String,
    pub(crate) alpn: Option<String>,
    pub(crate) chain_result: ChainResult,
    pub(crate) chain_error: Option<String>,
}

impl TlsInfo {
    /// Protocol version, e.g. `TLSv1_3`.
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Cipher suite name.
    #[must_use]
    pub fn cipher(&self) -> &str {
        &self.cipher
    }

    /// Negotiated ALPN protocol.
    #[must_use]
    pub fn alpn(&self) -> Option<&str> {
        self.alpn.as_deref()
    }

    /// Chain validation result: `Valid`, or `Overridden` when
    /// [`VerifyMode::AcceptInvalid`] let a failing chain through.
    #[must_use]
    pub fn chain_result(&self) -> &ChainResult {
        &self.chain_result
    }

    /// Why verification failed when the result is `Overridden`.
    #[must_use]
    pub fn chain_error(&self) -> Option<&str> {
        self.chain_error.as_deref()
    }
}

/// A boxed future, so the trait stays object-safe.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// What a successful handshake yields: the stream and what was negotiated.
pub type Handshake = Result<(Box<dyn Conn>, TlsInfo), ClientError>;

/// A pluggable TLS client implementation.
pub trait TlsBackend: Send + Sync {
    /// Runs a client handshake over `tcp`. The handshake is time-limited and
    /// fails with [`ClientError::Timeout`] when it stalls.
    fn connect<'a>(&'a self, tcp: TcpStream, cfg: TlsConnectConfig) -> BoxFuture<'a, Handshake>;
}
