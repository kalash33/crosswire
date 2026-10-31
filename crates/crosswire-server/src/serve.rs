//! `crosswire serve`: compose the CA, gate, transport, store and proxy.

use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

use crosswire_api::{
    bootstrap_token, router_with_fallback, ApiConfig, AppState, DEFAULT_BOOTSTRAP_TTL,
    DEFAULT_IDLE_TIMEOUT,
};
use crosswire_ca::{IssuerLimits, LeafIssuer};
use crosswire_client::{DestinationGate, RustlsBackend, Transport};
use crosswire_proxy::{
    Budget, Handler, Intercept, InterceptRules, Proxy, ProxyAuth, ProxyConfig, ProxyDeps,
    ProxyLimits,
};
use crosswire_scope::{Compiled, Scope};
use crosswire_store::{Project, Store, StoreConfig};
use crosswire_tools::Repeater;
use tokio::net::TcpListener;
use tokio::sync::watch;

use crate::api_cmd;
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
    let store = Arc::new(
        Store::open(project, StoreConfig::default())
            .map_err(|e| CliError::runtime("opening the store", e))?,
    );
    if store.open_report().unclean_gap.is_some() {
        tracing::warn!("the previous run did not shut down cleanly; a gap marker was recorded");
    }
    let served = serve_with_store(cfg, args, ca_dir, Arc::clone(&store)).await;
    // The store is always closed here, on every path out of `serve_with_store`
    // (success, early `?` error, or shutdown): see `close_store`.
    let closed = close_store(store);
    served.and(closed)
}

/// Closes the store cleanly through the one remaining owning handle.
///
/// By the time `serve_with_store` has returned, every clone it made is gone:
/// the API server is awaited inline (not detached), so its `AppState` — and the
/// per-connection clones graceful shutdown waits for — are dropped with it; the
/// repeater and intercept are its locals, dropped as it returns (or unwound on
/// an early error); and the only detached task (the shutdown signal) holds no
/// store handle. So this `Arc` is the sole owner and the unwrap always succeeds.
/// A non-unwrappable `Arc` would mean a future change leaked a store handle into
/// a surviving task; it is surfaced loudly rather than left to silently skip the
/// clean close.
fn close_store(store: Arc<Store>) -> Result<(), CliError> {
    match Arc::try_unwrap(store) {
        Ok(store) => store
            .close()
            .map_err(|e| CliError::runtime("closing the store", e)),
        Err(arc) => {
            tracing::error!(
                refs = Arc::strong_count(&arc),
                "the store was still referenced at shutdown; the project may be left unclean"
            );
            Err(CliError::runtime(
                "closing the store",
                "the store was still referenced at shutdown",
            ))
        }
    }
}

async fn serve_with_store(
    cfg: ProxyConfig,
    args: &ServeArgs,
    ca_dir: &Path,
    store: Arc<Store>,
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

    // One control handle for the serve-local writes (the stored scope and the
    // proxy address). The API opens its own; both are rare synchronous writes.
    let control = store
        .control()
        .map_err(|e| CliError::runtime("opening a control handle", e))?;

    // The scope shared by the proxy, the intercept and the repeater, loaded
    // from the store if present. Strict mode is the flag OR the persisted
    // setting. Intercept always starts disabled and with default rules: its
    // rules are deliberately NOT persisted or loaded (that is on the pre-0.1.0
    // backlog, pending Serialize/Deserialize on InterceptRules).
    let loaded_scope = api_cmd::load_scope(&control)?;
    let compiled = loaded_scope.compile().unwrap_or_else(|e| {
        tracing::warn!(error = %e, "the stored scope did not compile; starting with an empty scope");
        Scope::default()
            .compile()
            .expect("the empty scope compiles")
    });
    let scope: Arc<RwLock<Compiled>> = Arc::new(RwLock::new(compiled));
    let scope_doc: Arc<RwLock<Scope>> = Arc::new(RwLock::new(loaded_scope));
    // The flag forces strict on; otherwise the persisted setting decides.
    let strict = Arc::new(AtomicBool::new(
        args.enforce_scope_strictly || api_cmd::load_enforce_strict(&control)?,
    ));

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

    // The intercept shares the scope with the proxy and records through the
    // store sink. The proxy builds its own in-flight body budget internally
    // (`ProxyDeps` has no budget field), so the intercept gets a matching one;
    // the two draw on separate budgets.
    let limits: ProxyLimits = cfg.limits.clone();
    // Intercept starts disabled with default rules; rules are not persisted.
    let intercept = Arc::new(Intercept::new(
        &limits,
        Arc::clone(&scope),
        InterceptRules::default(),
        Arc::new(Budget::new(limits.body_budget)),
        store.sink(),
    ));

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
        handler: Arc::clone(&intercept) as Arc<dyn Handler>,
        scope: Arc::clone(&scope),
        resolve_override: None,
    };
    let proxy = Proxy::from_listener(cfg, listener, deps)
        .map_err(|e| CliError::runtime("starting the proxy", e))?;
    let addr = proxy.local_addr();
    audit("listener.bind", &addr.to_string())?;

    // Record the proxy address so `crosswire browser` can find it (readable
    // only while this process is not holding the project lock).
    control
        .setting_put(api_cmd::SETTING_PROXY_BIND, &addr.to_string())
        .map_err(|e| CliError::runtime("recording the proxy address", e))?;

    // The repeater dials through its own transport (mirroring --allow) and
    // records security-relevant sends to the audit log. A no-op hook here would
    // silently drop the out-of-scope-send audit requirement.
    let mut repeater_gate = DestinationGate::new(Vec::new());
    for (host, port) in &args.allow {
        repeater_gate.allow(host, *port);
    }
    let repeater_transport = Arc::new(Transport::new(
        Arc::new(RustlsBackend::webpki_roots()),
        Arc::new(repeater_gate),
    ));
    let audit_store = Arc::clone(&store);
    let repeater = Arc::new(Repeater::new(
        repeater_transport,
        store.sink(),
        Arc::clone(&scope),
        Arc::clone(&strict),
        Arc::new(move |action: &str, target: &str, result: &str| {
            let _ = audit_store.audit(ACTOR, action, target, result);
        }),
        limits.parse,
    ));

    // Bind the API (a separate socket) and build its shared state.
    let (api_listener, api_addr) = api_cmd::bind_api(args.api_bind).await?;
    let api_port = api_addr.port();
    let api_config = ApiConfig {
        bind: api_addr,
        idle_timeout: DEFAULT_IDLE_TIMEOUT,
        bootstrap_ttl: DEFAULT_BOOTSTRAP_TTL,
        proxy_addr: Some(addr),
    };
    let state = AppState::new(
        Arc::clone(&store),
        Arc::clone(&repeater),
        Arc::clone(&intercept),
        Arc::clone(&scope),
        scope_doc,
        Arc::clone(&ca),
        Arc::clone(&strict),
        api_port,
        &api_config,
    )
    .map_err(|e| CliError::runtime("building the API state", e))?;

    // Mint the single-use bootstrap token once and deliver it out of band.
    let token = bootstrap_token(&state);
    let url = api_cmd::bootstrap_url(api_port, &token);
    let bootstrap_line = api_cmd::deliver_bootstrap(args.bootstrap_file.as_deref(), &url)?;
    drop(control);

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
            .and_then(|()| writeln!(out, "api listening on {api_addr}"))
            .and_then(|()| writeln!(out, "{bootstrap_line}"))
            .and_then(|()| out.flush())
            .map_err(|e| CliError::runtime("writing stdout", e))?;
    }
    tracing::info!(%addr, %api_addr, "proxy and api listening");

    let counters = proxy.counters();
    let (tx, rx) = watch::channel(false);
    let mut rx_api = rx.clone();
    tokio::spawn(async move {
        signalled.await;
        tracing::info!("shutting down");
        let _ = tx.send(true);
    });

    // Serve the API with ConnectInfo (the auth reads the peer IP) under the
    // same shutdown signal as the proxy, and run both concurrently. The embedded
    // UI bundle is the router's fallback: non-`/api/v1` paths are the
    // static-asset class, served same-origin under the UI CSP, while every
    // `/api/v1` route stays gated.
    let api_service = router_with_fallback(state, crate::ui::service(api_port))
        .into_make_service_with_connect_info::<SocketAddr>();
    let api_server = axum::serve(api_listener, api_service).with_graceful_shutdown(async move {
        let _ = rx_api.changed().await;
    });

    // `join!` (not a detached spawn) means the API server future — and the
    // per-connection tasks graceful shutdown waits for — are complete and
    // dropped here, so the `AppState` (and its store clone) is gone.
    let (ran, api_res) = tokio::join!(proxy.run(rx), api_server);
    let ran = ran.map_err(|e| CliError::runtime("proxy", e));
    if let Err(e) = api_res {
        tracing::warn!(error = %e, "the API listener stopped with an error");
    }
    tracing::info!(
        accepted = counters.accepted.load(Ordering::Relaxed),
        exchanges = counters.exchanges.load(Ordering::Relaxed),
        tunnels = counters.tunnels.load(Ordering::Relaxed),
        "proxy stopped"
    );
    // Written before the store closes: the audit log is closed with it.
    let stopped = audit("listener.stop", &addr.to_string());
    // Drop every remaining store handle before returning, so the caller can
    // unwrap the sole `Arc` and close the store cleanly. `repeater` owns a store
    // clone (its audit hook); `intercept` holds only a sink, but is dropped for
    // clarity. The `audit` closure borrows `store`, and that borrow ends at its
    // last use just above.
    drop(repeater);
    drop(intercept);
    ran.and(stopped)
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
