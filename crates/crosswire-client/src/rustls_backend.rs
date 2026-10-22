//! The rustls implementation of [`TlsBackend`].

use std::sync::{Arc, Mutex};
use std::time::Duration;

use crosswire_model::ChainResult;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::WebPkiServerVerifier;
use rustls::crypto::{ring, CryptoProvider};
use rustls::{
    ClientConfig, DigitallySignedStruct, Error as TlsError, RootCertStore, SignatureScheme,
};
use rustls_pki_types::{CertificateDer, ServerName, UnixTime};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

use crate::error::ClientError;
use crate::tls::{BoxFuture, Handshake, TlsBackend, TlsConnectConfig, TlsInfo, VerifyMode};

type FailureSlot = Arc<Mutex<Option<String>>>;

/// Handshake time limit.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);

/// TLS via rustls (ring provider). TLS 1.2 is the minimum version.
#[derive(Clone, Debug)]
pub struct RustlsBackend {
    roots: Arc<RootCertStore>,
}

impl RustlsBackend {
    /// Trusts the bundled Mozilla (webpki) roots.
    #[must_use]
    pub fn webpki_roots() -> RustlsBackend {
        let mut roots = RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        RustlsBackend {
            roots: Arc::new(roots),
        }
    }

    /// Trusts the webpki roots plus extra DER certificates (for example a
    /// private CA or an upstream proxy trust anchor). A certificate that does
    /// not parse as a trust anchor is skipped, which only narrows trust.
    #[must_use]
    pub fn with_extra_roots(ders: Vec<Vec<u8>>) -> RustlsBackend {
        let mut roots = RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        for der in ders {
            let _ = roots.add(CertificateDer::from(der));
        }
        RustlsBackend {
            roots: Arc::new(roots),
        }
    }
}

/// Verifies exactly like webpki; on failure it records the error and lets
/// the handshake continue. Signature checks are never skipped.
#[derive(Debug)]
struct RecordingVerifier {
    inner: Arc<WebPkiServerVerifier>,
    failure: Arc<Mutex<Option<String>>>,
}

impl ServerCertVerifier for RecordingVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        match self.inner.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        ) {
            Ok(v) => Ok(v),
            Err(e) => {
                let mut slot = self.failure.lock().unwrap_or_else(|p| p.into_inner());
                *slot = Some(e.to_string());
                // The detailed variant is more useful to the user than Display.
                if let TlsError::InvalidCertificate(reason) = &e {
                    *slot = Some(format!("{reason:?}"));
                }
                Ok(ServerCertVerified::assertion())
            }
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

fn tls_err(e: impl std::fmt::Display) -> ClientError {
    ClientError::Tls(e.to_string())
}

impl RustlsBackend {
    fn build(&self, cfg: &TlsConnectConfig) -> Result<(ClientConfig, FailureSlot), ClientError> {
        let provider: Arc<CryptoProvider> = Arc::new(ring::default_provider());
        let inner = WebPkiServerVerifier::builder_with_provider(
            Arc::clone(&self.roots),
            Arc::clone(&provider),
        )
        .build()
        .map_err(tls_err)?;
        let failure = Arc::new(Mutex::new(None));
        let verifier: Arc<dyn ServerCertVerifier> = match cfg.verify {
            VerifyMode::WebPki => inner,
            VerifyMode::AcceptInvalid => Arc::new(RecordingVerifier {
                inner,
                failure: Arc::clone(&failure),
            }),
        };
        let builder = ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(tls_err)?
            .dangerous()
            .with_custom_certificate_verifier(verifier);
        let mut config = match &cfg.client_cert {
            Some((chain, key)) => builder
                .with_client_auth_cert(chain.clone(), key.clone_key())
                .map_err(tls_err)?,
            None => builder.with_no_client_auth(),
        };
        config.alpn_protocols = cfg.alpn.clone();
        Ok((config, failure))
    }

    async fn handshake(&self, tcp: TcpStream, cfg: TlsConnectConfig) -> Handshake {
        let name = cfg
            .sni
            .clone()
            .ok_or_else(|| ClientError::Tls("no server name for verification".into()))?;
        let (config, failure) = self.build(&cfg)?;
        let connector = TlsConnector::from(Arc::new(config));
        let stream = tokio::time::timeout(HANDSHAKE_TIMEOUT, connector.connect(name, tcp))
            .await
            .map_err(|_| ClientError::Timeout)?
            .map_err(tls_err)?;
        let (_, conn) = stream.get_ref();
        let version = conn
            .protocol_version()
            .map_or_else(String::new, |v| format!("{v:?}"));
        let cipher = conn
            .negotiated_cipher_suite()
            .map_or_else(String::new, |c| format!("{:?}", c.suite()));
        let alpn = conn
            .alpn_protocol()
            .map(|p| String::from_utf8_lossy(p).into_owned());
        let chain_error = failure.lock().unwrap_or_else(|p| p.into_inner()).take();
        let chain_result = if chain_error.is_some() {
            ChainResult::Overridden
        } else {
            ChainResult::Valid
        };
        let info = TlsInfo {
            version,
            cipher,
            alpn,
            chain_result,
            chain_error,
        };
        Ok((Box::new(stream), info))
    }
}

impl TlsBackend for RustlsBackend {
    fn connect<'a>(&'a self, tcp: TcpStream, cfg: TlsConnectConfig) -> BoxFuture<'a, Handshake> {
        Box::pin(self.handshake(tcp, cfg))
    }
}
