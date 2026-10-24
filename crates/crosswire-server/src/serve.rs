//! `crosswire serve`: compose the CA, gate, transport, store and proxy.

use std::io::Write;
use std::path::{Path, PathBuf};
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
    refuse_ca_dir_in_project(&ca_dir, &args.project)?;
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

    // Handlers first: a caller that reads the banner and then signals must
    // get a clean shutdown, not the default "terminate now".
    let signalled = shutdown_signal();
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
        signalled.await;
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

/// The CA private key must never live under a project directory (spec
/// 11.3): a project is shared and exported, the key is not. Both paths are
/// resolved (symlinks and `..` included, even when they do not exist yet)
/// before comparing.
fn refuse_ca_dir_in_project(ca_dir: &Path, project: &Path) -> Result<(), CliError> {
    let (ca, proj) = (resolved(ca_dir)?, resolved(project)?);
    if ca.starts_with(&proj) {
        return Err(CliError::Config(format!(
            "--ca-dir {} is inside the project {}; keep the CA key outside any project",
            ca_dir.display(),
            project.display()
        )));
    }
    Ok(())
}

/// `path` made absolute, with its longest existing prefix canonicalised
/// and the rest appended (`.` dropped, `..` applied).
fn resolved(path: &Path) -> Result<PathBuf, CliError> {
    use std::path::Component;
    let abs = std::path::absolute(path)
        .map_err(|e| CliError::Config(format!("cannot resolve {}: {e}", path.display())))?;
    let mut base = abs.clone();
    let mut rest = Vec::new();
    let canon = loop {
        if let Ok(c) = base.canonicalize() {
            break c;
        }
        match (base.file_name().map(ToOwned::to_owned), base.parent()) {
            (Some(name), Some(parent)) => {
                rest.push(name);
                base = parent.to_path_buf();
            }
            // `..` as the last component, or the root: resolve lexically.
            _ => match base.parent() {
                Some(parent) if base.ends_with("..") => {
                    rest.push("..".into());
                    base = parent.to_path_buf();
                }
                _ => break base,
            },
        }
    };
    let mut out = canon;
    for part in rest.iter().rev() {
        match Path::new(part).components().next() {
            Some(Component::ParentDir) => {
                out.pop();
            }
            Some(Component::CurDir) | None => {}
            _ => out.push(part),
        }
    }
    Ok(out)
}

/// `host:port`, bracketing an IPv6 host.
fn host_port(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

/// Installs the SIGINT (Ctrl-C) and, on Unix, SIGTERM handlers now, and
/// returns a future that resolves on either. A handler that cannot be
/// installed is logged and never fires, so the proxy keeps running.
#[cfg(unix)]
fn shutdown_signal() -> impl std::future::Future<Output = ()> {
    use tokio::signal::unix::{signal, SignalKind};
    let install = |kind: SignalKind, name: &'static str| match signal(kind) {
        Ok(s) => Some(s),
        Err(e) => {
            tracing::warn!(error = %e, "cannot listen for {name}");
            None
        }
    };
    let int = install(SignalKind::interrupt(), "SIGINT");
    let term = install(SignalKind::terminate(), "SIGTERM");
    async move {
        let wait = |s: Option<tokio::signal::unix::Signal>| async move {
            match s {
                Some(mut s) => {
                    s.recv().await;
                }
                None => std::future::pending::<()>().await,
            }
        };
        tokio::select! {
            () = wait(int) => {}
            () = wait(term) => {}
        }
    }
}

/// Resolves on Ctrl-C. On non-Unix platforms the handler is registered
/// when the future is first polled.
#[cfg(not(unix))]
fn shutdown_signal() -> impl std::future::Future<Output = ()> {
    async {
        if let Err(e) = tokio::signal::ctrl_c().await {
            tracing::warn!(error = %e, "cannot listen for Ctrl-C");
            std::future::pending::<()>().await;
        }
    }
}
