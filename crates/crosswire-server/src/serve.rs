//! `crosswire serve`: compose the CA, gate, transport, store and proxy.

use std::io::Write;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crosswire_ca::{IssuerLimits, LeafIssuer};
use crosswire_client::{DestinationGate, RustlsBackend, Transport};
use crosswire_proxy::{NoopHandler, Proxy, ProxyAuth, ProxyConfig, ProxyDeps, ProxyLimits};
use crosswire_store::{Project, Store, StoreConfig};
use tokio::net::TcpListener;
use tokio::sync::watch;

use crate::ca_cmd::{load_or_generate, resolve_ca_dir};
use crate::cli::ServeArgs;
use crate::error::CliError;

/// The actor recorded in the audit log for configuration given on the
/// command line.
const ACTOR: &str = "cli";

/// Runs `crosswire serve` until Ctrl-C (or SIGTERM).
///
/// # Errors
/// [`CliError::Config`] for an invalid configuration (checked before
/// anything is created); [`CliError::Runtime`] for failures to open the
/// project, load the CA, bind, audit or close.
pub fn run(args: ServeArgs) -> Result<(), CliError> {
    let cfg = proxy_config(&args)?;
    let ca_dir = resolve_ca_dir(args.ca_dir.clone())?;
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| CliError::runtime("starting the runtime", e))?;
    rt.block_on(serve(cfg, &args, &ca_dir))
}

/// Builds and validates the proxy configuration.
fn proxy_config(args: &ServeArgs) -> Result<ProxyConfig, CliError> {
    let mut cfg = ProxyConfig::new(args.proxy_bind);
    cfg.allow_non_loopback = args.allow_non_loopback;
    cfg.proxy_auth = match &args.proxy_auth {
        Some((user, pass)) => Some(
            ProxyAuth::new(user.clone(), pass.clone())
                .map_err(|e| CliError::Config(e.to_string()))?,
        ),
        None => None,
    };
    cfg.passthrough.clone_from(&args.passthrough);
    cfg.accept_invalid_cert = args
        .accept_invalid_cert
        .iter()
        .map(|h| h.to_ascii_lowercase())
        .collect();
    if let Some(cap) = args.capture_cap {
        cfg.limits = ProxyLimits::builder().capture_cap(cap).build();
    }
    cfg.validate()
        .map_err(|e| CliError::Config(e.to_string()))?;
    Ok(cfg)
}

async fn serve(cfg: ProxyConfig, args: &ServeArgs, ca_dir: &Path) -> Result<(), CliError> {
    let project = Project::open_or_create(&args.project).map_err(|e| {
        CliError::runtime(&format!("opening project {}", args.project.display()), e)
    })?;
    let store = Store::open(project, StoreConfig::default())
        .map_err(|e| CliError::runtime("opening the store", e))?;
    if store.open_report().unclean_gap.is_some() {
        tracing::warn!("the previous run did not shut down cleanly; a gap marker was recorded");
    }
    let served = serve_with_store(cfg, args, ca_dir, &store).await;
    let closed = store
        .close()
        .map_err(|e| CliError::runtime("closing the store", e));
    served.and(closed)
}

async fn serve_with_store(
    cfg: ProxyConfig,
    args: &ServeArgs,
    ca_dir: &Path,
    store: &Store,
) -> Result<(), CliError> {
    let audit = |action: &str, target: &str| {
        store
            .audit(ACTOR, action, target, "ok")
            .map_err(|e| CliError::runtime(&format!("writing audit row {action}"), e))
    };

    let loaded = load_or_generate(ca_dir)?;
    if loaded.generated {
        tracing::info!(dir = %ca_dir.display(), "generated a new CA");
        audit("ca.generate", &ca_dir.display().to_string())?;
    }

    // Bind first: the gate must protect the proxy's own port, which is only
    // known now when --proxy-bind asked for port 0.
    let listener = TcpListener::bind(cfg.bind).await.map_err(|e| {
        CliError::runtime(
            &format!("could not bind the proxy listener {}", cfg.bind),
            e,
        )
    })?;
    let own_port = listener
        .local_addr()
        .map_err(|e| CliError::runtime("reading the listener address", e))?
        .port();

    let mut gate = DestinationGate::new(vec![own_port]);
    for (host, port) in &args.allow {
        gate.allow(host, *port);
        audit("allow.add", &host_port(host, *port))?;
    }
    for host in &cfg.accept_invalid_cert {
        audit("cert.override.config", host)?;
    }

    let ca = Arc::new(loaded.ca);
    let deps = ProxyDeps {
        issuer: Arc::new(LeafIssuer::new(Arc::clone(&ca), IssuerLimits::default())),
        ca_der: ca.cert_der(),
        ca_pem: ca.cert_pem(),
        ca_spki: ca.spki_sha256_base64(),
        transport: Arc::new(Transport::new(
            Arc::new(RustlsBackend::webpki_roots()),
            Arc::new(gate),
        )),
        sink: store.sink(),
        handler: Arc::new(NoopHandler),
        resolve_override: None,
    };
    let proxy = Proxy::from_listener(cfg, listener, deps)
        .map_err(|e| CliError::runtime("starting the proxy", e))?;
    let addr = proxy.local_addr();
    audit("listener.bind", &addr.to_string())?;

    {
        let mut out = std::io::stdout().lock();
        writeln!(out, "proxy listening on {addr}")
            .and_then(|()| {
                writeln!(
                    out,
                    "ca page at http://crosswire/ (via the proxy) or http://{addr}/"
                )
            })
            .and_then(|()| out.flush())
            .map_err(|e| CliError::runtime("writing stdout", e))?;
    }
    tracing::info!(%addr, "proxy listening");

    let counters = proxy.counters();
    let (tx, rx) = watch::channel(false);
    tokio::spawn(async move {
        shutdown_signal().await;
        tracing::info!("shutting down");
        let _ = tx.send(true);
    });
    let ran = proxy
        .run(rx)
        .await
        .map_err(|e| CliError::runtime("proxy", e));
    tracing::info!(
        accepted = counters.accepted.load(Ordering::Relaxed),
        exchanges = counters.exchanges.load(Ordering::Relaxed),
        tunnels = counters.tunnels.load(Ordering::Relaxed),
        "proxy stopped"
    );
    // Written before the store closes: the audit log is closed with it.
    audit("listener.stop", &addr.to_string())?;
    ran
}

/// `host:port`, bracketing an IPv6 host.
fn host_port(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

/// Resolves on Ctrl-C (SIGINT) or, on Unix, SIGTERM. If no handler can be
/// installed it never resolves, so the proxy keeps running.
async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(e) = tokio::signal::ctrl_c().await {
            tracing::warn!(error = %e, "cannot listen for Ctrl-C");
            std::future::pending::<()>().await;
        }
    };
    #[cfg(unix)]
    let term = async {
        use tokio::signal::unix::{signal, SignalKind};
        match signal(SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(e) => {
                tracing::warn!(error = %e, "cannot listen for SIGTERM");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {}
        () = term => {}
    }
}
