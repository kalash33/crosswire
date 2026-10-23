//! CONNECT tunnels: SNI peek, passthrough, and TLS termination with leaves
//! from the Crosswire CA.

use std::net::IpAddr;
use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use crosswire_client::{Target, VerifyMode};
use crosswire_model::{ConnectionInfo, Marker, MarkerKind, TunnelReason};
use rustls::crypto::CryptoProvider;
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use rustls::ServerConfig;
use rustls_pki_types::ServerName;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio_rustls::TlsAcceptor;

use crate::listener::{ConnCtx, Shared};
use crate::record::{self, RecordCtx};
use crate::{front, relay, sni, tunnel, ProxyError};

/// How long the client has to send its ClientHello after the 200.
const SNI_PEEK_TIMEOUT: Duration = Duration::from_secs(5);

/// The answer to an accepted CONNECT.
const CONNECTION_ESTABLISHED: &[u8] = b"HTTP/1.1 200 Connection established\r\n\r\n";

/// The crypto provider shared by every per-connection server config.
fn provider() -> Arc<CryptoProvider> {
    static PROVIDER: OnceLock<Arc<CryptoProvider>> = OnceLock::new();
    Arc::clone(PROVIDER.get_or_init(|| Arc::new(rustls::crypto::ring::default_provider())))
}

/// Serves one leaf whatever the ClientHello asks for: the leaf was chosen
/// from the peeked SNI (or the CONNECT authority) before the handshake.
#[derive(Debug)]
struct FixedCert(Arc<CertifiedKey>);

impl ResolvesServerCert for FixedCert {
    fn resolve(&self, _hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        Some(Arc::clone(&self.0))
    }
}

/// Whether `sni` is on the passthrough list: an exact host, or under a
/// `.suffix` rule (which does not match the apex). Case-insensitive; a
/// trailing dot on `sni` is ignored.
pub(crate) fn passthrough_matches(list: &[String], sni: &str) -> bool {
    let sni = sni.trim_end_matches('.');
    list.iter().any(|rule| {
        if rule.starts_with('.') {
            sni.len() > rule.len()
                && sni.is_char_boundary(sni.len() - rule.len())
                && sni[sni.len() - rule.len()..].eq_ignore_ascii_case(rule)
        } else {
            sni.eq_ignore_ascii_case(rule)
        }
    })
}

/// The name a leaf is issued for: an IP literal (brackets stripped) gets
/// an IP SAN, anything else a DNS SAN.
fn leaf_name(host: &str) -> Result<ServerName<'static>, ProxyError> {
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = bare.parse::<IpAddr>() {
        return Ok(ServerName::IpAddress(ip.into()));
    }
    ServerName::try_from(host.to_string())
        .map_err(|_| ProxyError::Protocol(format!("invalid server name {host:?}")))
}

/// The host the upstream connection dials: the requested authority, or
/// the test-only override's address for it.
fn dial_host(shared: &Shared, authority: &str) -> String {
    shared
        .deps
        .resolve_override
        .as_ref()
        .and_then(|f| f(authority))
        .map_or_else(|| authority.to_string(), |ip| ip.to_string())
}

/// `AcceptInvalid` only for hosts on `accept_invalid_cert`.
fn verify_mode(shared: &Shared, authority: &str) -> VerifyMode {
    let host = authority.trim_end_matches('.');
    if shared
        .cfg
        .accept_invalid_cert
        .iter()
        .any(|h| h.eq_ignore_ascii_case(host))
    {
        VerifyMode::AcceptInvalid
    } else {
        VerifyMode::WebPki
    }
}

/// Answers a CONNECT: writes the 200, peeks the SNI, then either tunnels
/// to the upstream unread (passthrough) or terminates TLS with a leaf for
/// the SNI (or the authority) and hands the decrypted stream to the relay.
pub(crate) async fn serve_connect(
    mut client: TcpStream,
    ctx: ConnCtx,
    authority: String,
    port: u16,
) -> Result<(), ProxyError> {
    client
        .write_all(CONNECTION_ESTABLISHED)
        .await
        .map_err(ProxyError::Io)?;
    let sni = sni::peek_sni(&client, SNI_PEEK_TIMEOUT).await?;
    if let Some(name) = &sni {
        if !front::valid_authority(name) {
            return Err(ProxyError::Protocol(format!(
                "invalid SNI {:?}",
                crate::generated::cap(name)
            )));
        }
    }
    let name = sni.as_deref().unwrap_or(&authority).to_string();
    if passthrough_matches(&ctx.shared.cfg.passthrough, &name) {
        passthrough(client, ctx, &authority, port, sni).await
    } else {
        terminate(client, ctx, &authority, port, &name).await
    }
}

/// Tunnels the client to `authority:port` without reading the TLS inside.
async fn passthrough(
    client: TcpStream,
    mut ctx: ConnCtx,
    authority: &str,
    port: u16,
    sni: Option<String>,
) -> Result<(), ProxyError> {
    let shared = Arc::clone(&ctx.shared);
    let target = Target::plain(&dial_host(&shared, authority), port);
    // On failure the client is dropped (closed); Task 7 adds a 502.
    let upstream = shared.deps.transport.connect(&target).await?;
    ctx.pending.release();
    shared.counters.tunnels.fetch_add(1, Ordering::Relaxed);
    let (io, up_info, _, _) = upstream.into_parts();
    let mut info = ConnectionInfo::new(authority, port)
        .with_client_addr(ctx.peer.to_string())
        .with_conn(up_info.conn_id(), false);
    if let Some(peer) = up_info.peer_addr() {
        info = info.with_peer_addr(peer);
    }
    if let Some(sni) = sni {
        info = info.with_sni(sni);
    }
    let rec = RecordCtx {
        sink: Arc::clone(&shared.deps.sink),
        counters: Arc::clone(&shared.counters),
        info,
    };
    let stats = tunnel::run(
        Box::new(client),
        io,
        TunnelReason::SniPassthrough,
        shared.cfg.limits.tunnel_idle,
        rec,
    )
    .await;
    tracing::debug!(%authority, port, ?stats, "passthrough tunnel closed");
    drop(ctx);
    Ok(())
}

/// Accepts the client's TLS with a leaf for `leaf_host`, then hands the
/// stream to the relay with the upstream target.
async fn terminate(
    client: TcpStream,
    mut ctx: ConnCtx,
    authority: &str,
    port: u16,
    leaf_host: &str,
) -> Result<(), ProxyError> {
    let shared = Arc::clone(&ctx.shared);
    let key = shared
        .deps
        .issuer
        .issue(&leaf_name(leaf_host)?)
        .map_err(|e| ProxyError::Tls(format!("leaf for {leaf_host}: {e}")))?;
    let mut cfg = ServerConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
        .map_err(|e| ProxyError::Tls(e.to_string()))?
        .with_no_client_auth()
        .with_cert_resolver(Arc::new(FixedCert(key)));
    cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
    let acceptor = TlsAcceptor::from(Arc::new(cfg));
    let tls = match tokio::time::timeout(shared.cfg.limits.tls_handshake, acceptor.accept(client))
        .await
    {
        Err(_) => return Err(ProxyError::Timeout("client TLS handshake")),
        Ok(Err(e)) => {
            record_rejection(&shared, leaf_host);
            return Err(ProxyError::Tls(format!("client handshake: {e}")));
        }
        Ok(Ok(tls)) => tls,
    };
    ctx.pending.release();
    let target = Target::tls(
        &dial_host(&shared, authority),
        port,
        leaf_host,
        verify_mode(&shared, authority),
    );
    relay::run_stub(Box::new(tls), target, ctx).await
}

/// Counts a rejected client handshake and, once per window when the host
/// reaches the threshold, records an error marker. No auto-passthrough.
fn record_rejection(shared: &Shared, host: &str) {
    shared
        .counters
        .handshake_rejections
        .fetch_add(1, Ordering::Relaxed);
    if shared.rejections.record(host, Instant::now()) {
        tracing::warn!(%host, "client keeps rejecting the Crosswire certificate");
        record::submit_marker(
            &shared.deps.sink,
            &shared.counters,
            Marker::new(
                MarkerKind::Error,
                format!(
                    "Browser rejected the Crosswire certificate for {host} — is the CA trusted?"
                ),
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passthrough_suffix_rule() {
        let list = vec!["pass.test".to_string(), ".example.com".to_string()];
        assert!(passthrough_matches(&list, "pass.test"));
        assert!(passthrough_matches(&list, "PASS.Test"));
        assert!(passthrough_matches(&list, "pass.test."));
        assert!(!passthrough_matches(&list, "x.pass.test"));
        assert!(passthrough_matches(&list, "a.example.com"));
        assert!(passthrough_matches(&list, "a.b.EXAMPLE.com"));
        assert!(!passthrough_matches(&list, "example.com"));
        assert!(!passthrough_matches(&list, "badexample.com"));
        assert!(!passthrough_matches(&[], "pass.test"));
    }
}
