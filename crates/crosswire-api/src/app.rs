//! The application state and the router that ties every route together.

use std::net::SocketAddr;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use axum::middleware::from_fn_with_state;
use axum::Router;
use crosswire_ca::Ca;
use crosswire_proxy::Intercept;
use crosswire_scope::{Compiled, Scope};
use crosswire_store::{Control, Store, StoreError};
use crosswire_tools::Repeater;
use tower_http::trace::TraceLayer;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::auth::{gate, RateLimiter};
use crate::session::SessionStore;
use crate::tokens::BootstrapStore;

/// The default session idle timeout (30 minutes).
pub const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// The default bootstrap-token lifetime (60 seconds).
pub const DEFAULT_BOOTSTRAP_TTL: Duration = Duration::from_secs(60);

/// Static configuration for the API listener and its credential lifetimes.
///
/// `bind` is honoured by the server (Task 7); the API itself serves loopback
/// hosts only. The timeouts are injectable so tests can shorten them.
#[derive(Clone, Debug)]
pub struct ApiConfig {
    /// The loopback address to bind.
    pub bind: SocketAddr,
    /// How long a session may be idle before it expires (default 30 min).
    pub idle_timeout: Duration,
    /// How long a bootstrap token stays valid (default 60 s).
    pub bootstrap_ttl: Duration,
}

impl Default for ApiConfig {
    fn default() -> ApiConfig {
        ApiConfig {
            bind: SocketAddr::from(([127, 0, 0, 1], 0)),
            idle_timeout: DEFAULT_IDLE_TIMEOUT,
            bootstrap_ttl: DEFAULT_BOOTSTRAP_TTL,
        }
    }
}

/// Everything the handlers share. Cheap to clone (every field is an `Arc`,
/// a handle, or `Copy`).
#[derive(Clone)]
pub struct AppState {
    /// The durable exchange store.
    pub store: Arc<Store>,
    /// The repeater, for `POST /repeater/send`.
    pub repeater: Arc<Repeater>,
    /// The request-intercept service.
    pub intercept: Arc<Intercept>,
    /// The live compiled scope the proxy, repeater and intercept read.
    pub scope: Arc<RwLock<Compiled>>,
    /// The scope source document `GET /scope` returns and `PUT /scope` sets.
    pub scope_doc: Arc<RwLock<Scope>>,
    /// The local certificate authority, for the CA routes.
    pub ca: Arc<Ca>,
    /// In-memory operator sessions.
    pub sessions: SessionStore,
    /// The single-use bootstrap token.
    pub bootstrap: BootstrapStore,
    /// The listener port, which names the session cookie and the origin.
    pub port: u16,
    /// Whether out-of-scope sends are refused outright.
    pub strict: Arc<AtomicBool>,
    /// One shared control handle (its connection is `!Sync`, so it lives
    /// behind a mutex and is locked only for short synchronous writes).
    pub control: Arc<Mutex<Control>>,
    /// The session idle timeout.
    pub idle_timeout: Duration,
    /// The bootstrap-token lifetime.
    pub bootstrap_ttl: Duration,
    /// Per-peer authentication-failure rate limiter.
    pub auth_limiter: RateLimiter,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("port", &self.port)
            .field("sessions", &self.sessions.len())
            .finish_non_exhaustive()
    }
}

impl AppState {
    /// Builds the shared state, opening one control handle on `store`.
    ///
    /// # Errors
    /// [`StoreError`] when the control connection cannot be opened.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        store: Arc<Store>,
        repeater: Arc<Repeater>,
        intercept: Arc<Intercept>,
        scope: Arc<RwLock<Compiled>>,
        scope_doc: Arc<RwLock<Scope>>,
        ca: Arc<Ca>,
        strict: Arc<AtomicBool>,
        port: u16,
        config: &ApiConfig,
    ) -> Result<AppState, StoreError> {
        let control = Arc::new(Mutex::new(store.control()?));
        Ok(AppState {
            store,
            repeater,
            intercept,
            scope,
            scope_doc,
            ca,
            sessions: SessionStore::new(),
            bootstrap: BootstrapStore::new(),
            port,
            strict,
            control,
            idle_timeout: config.idle_timeout,
            bootstrap_ttl: config.bootstrap_ttl,
            auth_limiter: RateLimiter::new(),
        })
    }
}

/// Mints the single-use bootstrap token (256-bit secret, 60 s expiry by
/// default), accepted only by `POST /api/v1/session`. The server (Task 7)
/// delivers it out of band; it is never passed on argv.
#[must_use]
pub fn bootstrap_token(state: &AppState) -> String {
    state.bootstrap.mint(state.bootstrap_ttl)
}

/// Every route, with no middleware applied (used for the router and for the
/// OpenAPI document).
pub(crate) fn api_routes() -> OpenApiRouter<AppState> {
    use crate::routes::{exchanges, intercept, misc, repeater, scope, tokens};
    OpenApiRouter::new()
        // Unauthenticated.
        .routes(routes!(misc::health))
        .routes(routes!(misc::openapi_json))
        .routes(routes!(misc::ca_der))
        .routes(routes!(misc::ca_pem))
        .routes(routes!(misc::create_session))
        // Read.
        .routes(routes!(exchanges::list_exchanges))
        .routes(routes!(exchanges::get_exchange, exchanges::patch_exchange))
        .routes(routes!(exchanges::get_body))
        .routes(routes!(exchanges::get_curl))
        .routes(routes!(exchanges::get_raw))
        .routes(routes!(exchanges::list_markers))
        .routes(routes!(exchanges::get_counts))
        .routes(routes!(scope::get_scope, scope::put_scope))
        .routes(routes!(scope::import_scope))
        .routes(routes!(scope::set_enforce_strict))
        .routes(routes!(intercept::get_intercept))
        .routes(routes!(intercept::get_held_bytes))
        .routes(routes!(intercept::put_enabled))
        .routes(routes!(intercept::put_rules))
        .routes(routes!(intercept::forward_held))
        .routes(routes!(intercept::drop_held))
        .routes(routes!(intercept::forward_all))
        .routes(routes!(repeater::send_request))
        .routes(routes!(repeater::list_tabs))
        .routes(routes!(
            repeater::get_tab,
            repeater::put_tab,
            repeater::delete_tab
        ))
        .routes(routes!(tokens::list_tokens, tokens::create_token))
        .routes(routes!(tokens::revoke_token))
        // WebSocket (authenticates inside the handler).
        .routes(routes!(crate::events::events_ws))
}

/// Applies the gate and the trace layer to a built route set, then supplies the
/// state. The gate and trace layers wrap whatever routes (and fallback) are
/// already present, so a fallback must be attached *before* this is called.
///
/// The trace layer records method and path only, at debug level; it never
/// records request or response bytes, tokens or cookies.
fn finish(router: Router<AppState>, state: AppState) -> Router {
    router
        .layer(from_fn_with_state(state.clone(), gate))
        // Record method and path only (never the query string, which can
        // carry filters, and never bodies, tokens or cookies).
        .layer(
            TraceLayer::new_for_http().make_span_with(|req: &axum::extract::Request| {
                tracing::debug_span!(
                    "request",
                    method = %req.method(),
                    path = %req.uri().path(),
                )
            }),
        )
        .with_state(state)
}

/// Builds the API router from the shared state. Unmatched routes fall through to
/// the default `404`.
pub fn router(state: AppState) -> Router {
    let (router, _api) = api_routes().split_for_parts();
    finish(router, state)
}

/// Builds the API router with `fallback` serving every unmatched route.
///
/// `crosswire-server` passes its embedded-UI service here so non-`/api/v1`
/// paths reach the UI bundle. The fallback is attached to the route set
/// *before* the gate layer, so the gate (Host check and the static-asset
/// allowlist in `is_unauthenticated`) covers the fallback just as
/// it covers the API routes: a non-loopback `Host` is still `421`, static assets
/// pass without a credential, and `/api/v1` routes stay gated. This crate must
/// not depend on `crosswire-server` (that would be circular), so the server
/// injects the service rather than this crate constructing it.
pub fn router_with_fallback<S>(state: AppState, fallback: S) -> Router
where
    S: tower::Service<axum::extract::Request, Error = std::convert::Infallible>
        + Clone
        + Send
        + Sync
        + 'static,
    S::Response: axum::response::IntoResponse,
    S::Future: Send + 'static,
{
    let (router, _api) = api_routes().split_for_parts();
    finish(router.fallback_service(fallback), state)
}
