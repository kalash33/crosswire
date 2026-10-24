//! The accept loop, connection caps and per-connection dispatch.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::Duration;

use base64::Engine as _;
use bytes::Bytes;
use crosswire_ca::LeafIssuer;
use crosswire_client::Transport;
use crosswire_h1::RawMessage;
use crosswire_model::{ConnectionInfo, ExchangeSink};
use crosswire_scope::Compiled;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio::task::JoinSet;

use crate::capture::Budget;
use crate::front::{self, FirstRequest};
use crate::record::{Recorder, RecorderThread};
use crate::rejections::RejectionTracker;
use crate::{generated, info_page, Counters, Handler, ProxyConfig, ProxyError};
use crate::{mitm, relay};

/// How long [`Proxy::run`] waits for live connections after shutdown.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);
/// How long a refused or answered connection is drained before closing, so
/// unread request bytes do not turn the close into a reset that destroys the
/// response.
const LINGER: Duration = Duration::from_secs(2);
/// Most bytes drained while lingering.
const LINGER_BYTES: usize = 256 * 1024;

/// Test-only host-to-address mapping (see [`ProxyDeps::resolve_override`]).
#[doc(hidden)]
pub type ResolveOverride = Arc<dyn Fn(&str) -> Option<IpAddr> + Send + Sync>;

/// What the proxy needs from the rest of the program.
pub struct ProxyDeps {
    /// Issues the leaf certificates presented to clients.
    pub issuer: Arc<LeafIssuer>,
    /// The CA certificate, DER, served at `/ca.der`.
    pub ca_der: Vec<u8>,
    /// The CA certificate, PEM, served at `/ca.pem`.
    pub ca_pem: String,
    /// SHA-256 of the CA's SubjectPublicKeyInfo, base64, served at
    /// `/ca.spki`.
    pub ca_spki: String,
    /// Connects to upstream servers.
    pub transport: Arc<Transport>,
    /// Where exchanges and markers are recorded.
    pub sink: Arc<dyn ExchangeSink>,
    /// Observes and steers traffic.
    pub handler: Arc<dyn Handler>,
    /// The project scope, evaluated for every exchange so history records a
    /// scope verdict (and so history can be filtered by scope). Default an
    /// empty compiled scope (everything `Empty`).
    pub scope: Arc<RwLock<Compiled>>,
    /// Test-only wiring: maps a requested host to a fixed address before
    /// the upstream connection, so tests can reach in-process servers under
    /// made-up names. The host is replaced by the address; the SNI and
    /// recorded target stay the requested name. Leave `None` in production.
    #[doc(hidden)]
    pub resolve_override: Option<ResolveOverride>,
}

/// State shared by the accept loop and every connection.
pub(crate) struct Shared {
    pub(crate) cfg: ProxyConfig,
    pub(crate) deps: ProxyDeps,
    pub(crate) counters: Arc<Counters>,
    pub(crate) local_addr: SocketAddr,
    /// `base64(user:pass)` when proxy auth is configured.
    pub(crate) expected_auth: Option<Vec<u8>>,
    /// Client handshake rejections per host.
    pub(crate) rejections: RejectionTracker,
    /// The in-flight body budget (`body_budget`) every capture draws from.
    pub(crate) budget: Arc<Budget>,
    /// Builds and submits what the relays and tunnels carried.
    pub(crate) recorder: Recorder,
    active: AtomicUsize,
    pending_connect: AtomicUsize,
    per_ip: Mutex<HashMap<IpAddr, usize>>,
}

/// Holds one of `max_connections` (and, for a non-loopback peer, one of its
/// IP's `max_per_client_ip`) until dropped.
pub(crate) struct ConnSlot {
    shared: Arc<Shared>,
    ip: Option<IpAddr>,
}

impl Drop for ConnSlot {
    fn drop(&mut self) {
        self.shared.active.fetch_sub(1, Ordering::Relaxed);
        if let Some(ip) = self.ip {
            let mut map = self
                .shared
                .per_ip
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if let Some(n) = map.get_mut(&ip) {
                *n -= 1;
                if *n == 0 {
                    map.remove(&ip);
                }
            }
        }
    }
}

/// Holds one of `max_pending_connect` from accept until the tunnel is
/// established ([`PendingConnect::release`]) or the connection ends.
pub(crate) struct PendingConnect {
    shared: Option<Arc<Shared>>,
}

impl PendingConnect {
    /// Gives the slot back; later calls do nothing.
    pub(crate) fn release(&mut self) {
        if let Some(shared) = self.shared.take() {
            shared.pending_connect.fetch_sub(1, Ordering::Relaxed);
        }
    }
}

impl Drop for PendingConnect {
    fn drop(&mut self) {
        self.release();
    }
}

/// One admitted client connection, handed to the tunnel or relay alongside
/// the client stream itself (which moves into TLS, so it is kept apart).
pub(crate) struct ConnCtx {
    /// The client's address.
    pub(crate) peer: SocketAddr,
    /// Proxy-wide state.
    pub(crate) shared: Arc<Shared>,
    /// Released once the tunnel is established: after the client TLS
    /// handshake, or once a passthrough tunnel's upstream is connected.
    pub(crate) pending: PendingConnect,
    /// Released when the connection ends.
    pub(crate) _slot: ConnSlot,
}

/// A bound proxy listener. Create with [`Proxy::bind`], then [`Proxy::run`].
pub struct Proxy {
    listener: TcpListener,
    shared: Arc<Shared>,
    recorder: RecorderThread,
}

impl std::fmt::Debug for Proxy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Proxy")
            .field("local_addr", &self.shared.local_addr)
            .finish_non_exhaustive()
    }
}

impl Proxy {
    /// Validates `cfg` and binds its listen address.
    ///
    /// # Errors
    /// Whatever [`ProxyConfig::validate`] returns, [`ProxyError::Bind`], or
    /// [`ProxyError::Io`] if the recorder thread cannot be started.
    pub async fn bind(cfg: ProxyConfig, deps: ProxyDeps) -> Result<Proxy, ProxyError> {
        cfg.validate()?;
        let listener = TcpListener::bind(cfg.bind)
            .await
            .map_err(ProxyError::Bind)?;
        Proxy::from_listener(cfg, listener, deps)
    }

    /// Serves on an already-bound `listener`, for callers that need the
    /// bound port before building `deps` (the destination gate protects the
    /// proxy's own port, which is only known after binding port 0).
    /// `cfg.bind` is replaced by the listener's address before validating.
    ///
    /// # Errors
    /// Whatever [`ProxyConfig::validate`] returns, [`ProxyError::Bind`]
    /// if the listener's address cannot be read, or [`ProxyError::Io`] if
    /// the recorder thread cannot be started.
    pub fn from_listener(
        mut cfg: ProxyConfig,
        listener: TcpListener,
        deps: ProxyDeps,
    ) -> Result<Proxy, ProxyError> {
        let local_addr = listener.local_addr().map_err(ProxyError::Bind)?;
        cfg.bind = local_addr;
        cfg.validate()?;
        let cfg_rejections = (
            cfg.limits.rejections_per_window,
            cfg.limits.rejection_window,
        );
        let expected_auth = cfg.proxy_auth.as_ref().map(|a| {
            base64::engine::general_purpose::STANDARD
                .encode(format!("{}:{}", a.user(), a.pass()))
                .into_bytes()
        });
        let counters = Arc::new(Counters::default());
        let budget = Arc::new(Budget::new(cfg.limits.body_budget));
        let (recorder, recorder_thread) = Recorder::start(
            Arc::clone(&deps.sink),
            Arc::clone(&counters),
            cfg.limits.parse,
        )
        .map_err(ProxyError::Io)?;
        Ok(Proxy {
            listener,
            recorder: recorder_thread,
            shared: Arc::new(Shared {
                cfg,
                deps,
                counters,
                budget,
                recorder,
                local_addr,
                expected_auth,
                rejections: RejectionTracker::new(cfg_rejections.0, cfg_rejections.1),
                active: AtomicUsize::new(0),
                pending_connect: AtomicUsize::new(0),
                per_ip: Mutex::new(HashMap::new()),
            }),
        })
    }

    /// The address the listener is bound to.
    #[must_use]
    pub fn local_addr(&self) -> SocketAddr {
        self.shared.local_addr
    }

    /// The proxy's counters.
    #[must_use]
    pub fn counters(&self) -> Arc<Counters> {
        Arc::clone(&self.shared.counters)
    }

    /// Accepts and serves connections until `shutdown` becomes `true` (or
    /// its sender is dropped). Then it stops accepting, waits up to five
    /// seconds for live connections before aborting them, and returns once
    /// the recorder thread has handed everything recorded to the sink.
    ///
    /// # Errors
    /// None today; accept errors are logged and retried.
    pub async fn run(self, mut shutdown: watch::Receiver<bool>) -> Result<(), ProxyError> {
        let mut tasks = JoinSet::new();
        if !*shutdown.borrow_and_update() {
            loop {
                tokio::select! {
                    biased;
                    changed = shutdown.changed() => {
                        if changed.is_err() || *shutdown.borrow_and_update() {
                            break;
                        }
                    }
                    Some(_) = tasks.join_next(), if !tasks.is_empty() => {}
                    accepted = self.listener.accept() => match accepted {
                        Ok((tcp, peer)) => self.admit(tcp, peer, &mut tasks),
                        Err(e) => {
                            tracing::warn!(error = %e, "accept failed");
                            // Out of descriptors, usually: back off briefly.
                            tokio::time::sleep(Duration::from_millis(50)).await;
                        }
                    },
                }
            }
        }
        drop(self.listener);
        let drain = async { while tasks.join_next().await.is_some() {} };
        if tokio::time::timeout(SHUTDOWN_GRACE, drain).await.is_err() {
            tracing::info!(remaining = tasks.len(), "aborting live connections");
            tasks.abort_all();
            while tasks.join_next().await.is_some() {}
        }
        self.recorder.stop().await;
        Ok(())
    }

    /// Applies the caps, then spawns the connection task.
    fn admit(&self, tcp: TcpStream, peer: SocketAddr, tasks: &mut JoinSet<()>) {
        let shared = &self.shared;
        let limits = &shared.cfg.limits;
        let counters = &shared.counters;
        // Only this task increments, so check-then-increment does not race.
        if shared.active.load(Ordering::Relaxed) >= limits.max_connections {
            counters
                .refused_max_connections
                .fetch_add(1, Ordering::Relaxed);
            tracing::debug!(%peer, "refused: max connections");
            return;
        }
        // An IPv4-mapped IPv6 peer (::ffff:127.0.0.1 on a dual-stack
        // listener) must get the IPv4 rules: loopback exemption and one
        // per-IP bucket per address, whatever its spelling.
        let peer_ip = peer.ip().to_canonical();
        let ip = (!peer_ip.is_loopback()).then_some(peer_ip);
        if let Some(ip) = ip {
            let map = shared.per_ip.lock().unwrap_or_else(PoisonError::into_inner);
            if map.get(&ip).copied().unwrap_or(0) >= limits.max_per_client_ip {
                counters.refused_per_ip.fetch_add(1, Ordering::Relaxed);
                tracing::debug!(%peer, "refused: per-client-IP cap");
                return;
            }
        }
        if shared.pending_connect.load(Ordering::Relaxed) >= limits.max_pending_connect {
            counters
                .refused_pending_connect
                .fetch_add(1, Ordering::Relaxed);
            tracing::debug!(%peer, "refused: pending CONNECT cap");
            return;
        }
        shared.active.fetch_add(1, Ordering::Relaxed);
        if let Some(ip) = ip {
            *shared
                .per_ip
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .entry(ip)
                .or_insert(0) += 1;
        }
        shared.pending_connect.fetch_add(1, Ordering::Relaxed);
        counters.accepted.fetch_add(1, Ordering::Relaxed);
        let _ = tcp.set_nodelay(true);
        let ctx = ConnCtx {
            peer,
            shared: Arc::clone(shared),
            pending: PendingConnect {
                shared: Some(Arc::clone(shared)),
            },
            _slot: ConnSlot {
                shared: Arc::clone(shared),
                ip,
            },
        };
        tasks.spawn(serve_connection(tcp, ctx));
    }
}

/// Reads the first request and dispatches it.
async fn serve_connection(mut client: TcpStream, mut ctx: ConnCtx) {
    let shared = Arc::clone(&ctx.shared);
    let read = front::read_first_request(&mut client, &shared.cfg.limits, shared.local_addr).await;
    let first = match read {
        Ok((first, _head_bytes)) => first,
        Err(e) => {
            tracing::debug!(peer = %ctx.peer, error = %e, "no first request");
            return;
        }
    };
    if !matches!(first, FirstRequest::Connect { .. }) {
        ctx.pending.release();
    }
    if let (
        Some(expected),
        FirstRequest::Connect { head, .. } | FirstRequest::Absolute { head, .. },
    ) = (&shared.expected_auth, &first)
    {
        if !front::proxy_auth_ok(head, expected) {
            shared
                .counters
                .auth_failures
                .fetch_add(1, Ordering::Relaxed);
            tracing::info!(peer = %ctx.peer, "proxy authentication failed");
            respond_and_close(client, &front::proxy_auth_required()).await;
            return;
        }
    }
    match first {
        FirstRequest::Local { head } => {
            let resp = info_page::respond(
                front::request_method(&head),
                &front::request_path(&head),
                &shared.deps,
            );
            respond_and_close(client, &resp).await;
        }
        FirstRequest::Refused { status, reason } => {
            let conn = ConnectionInfo::new("crosswire", shared.local_addr.port());
            let (resp, _) = generated::generated(status, reason_phrase(status), reason, conn);
            respond_and_close(client, &resp).await;
        }
        FirstRequest::Connect {
            authority, port, ..
        } => {
            if let Err(e) = mitm::serve_connect(client, ctx, authority, port).await {
                tracing::debug!(error = %e, "CONNECT ended");
            }
        }
        FirstRequest::Absolute {
            host,
            port,
            tls,
            head,
            leftover,
        } => {
            if let Err(e) = serve_plain(client, ctx, host, port, tls, head, leftover).await {
                tracing::debug!(error = %e, "relay ended");
            }
        }
    }
}

fn reason_phrase(status: u16) -> &'static str {
    match status {
        400 => "Bad Request",
        404 => "Not Found",
        407 => "Proxy Authentication Required",
        431 => "Request Header Fields Too Large",
        502 => "Bad Gateway",
        504 => "Gateway Timeout",
        _ => "Error",
    }
}

/// Writes `resp`, half-closes, and drains what the client still sends (for
/// a bounded time) so the close does not reset the response away.
pub(crate) async fn respond_and_close(mut tcp: TcpStream, resp: &[u8]) {
    if tcp.write_all(resp).await.is_err() || tcp.shutdown().await.is_err() {
        return;
    }
    linger(&mut tcp).await;
}

/// Reads and discards what a half-closed client still sends, for at most
/// two seconds and 256 KiB, so closing does not turn unread request bytes
/// into a reset that destroys the response just written.
pub(crate) async fn linger<R>(io: &mut R)
where
    R: AsyncRead + Unpin + ?Sized,
{
    let drain = async {
        let mut buf = [0u8; 8 * 1024];
        let mut total = 0;
        while total < LINGER_BYTES {
            match io.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => total += n,
            }
        }
    };
    let _ = tokio::time::timeout(LINGER, drain).await;
}

/// Relays an absolute-form request and whatever follows it on the
/// connection.
async fn serve_plain(
    client: TcpStream,
    ctx: ConnCtx,
    host: String,
    port: u16,
    tls: bool,
    head: RawMessage,
    leftover: Bytes,
) -> Result<(), ProxyError> {
    let dest = relay::Dest::absolute(&ctx.shared, &host, port, tls);
    relay::run(Box::new(client), Some((head, leftover)), dest, ctx).await
}
