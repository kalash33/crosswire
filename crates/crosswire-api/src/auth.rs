//! Authentication, authorisation, cookie isolation and rate limiting.
//!
//! Middleware order (spec §11.2): Host check (`421`) → route allowlist →
//! credential (session cookie with the same-origin/`Origin` check, or a bearer
//! token looked up by the SHA-256 of its secret) → scope check
//! (`403 insufficient_scope`) → idle-expiry refresh. The API never emits any
//! `Access-Control-*` header.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use crosswire_store::TokenScope;

use crate::app::AppState;
use crate::error::ApiError;
use crate::tokens::token_hash;

/// How many authentication failures a single peer may make per window.
const AUTH_FAIL_MAX: usize = 10;
/// The rate-limit window.
const RATE_WINDOW: Duration = Duration::from_secs(60);

/// A required or granted capability.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Read recorded exchanges and project state.
    Read,
    /// Send and replay requests, edit drafts, resolve held requests.
    Send,
    /// Administer the project: scope, tokens, settings, intercept config.
    Admin,
}

/// The capabilities a credential grants. `admin` implies `send`, which
/// implies `read`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Scopes {
    read: bool,
    send: bool,
    admin: bool,
}

impl Scopes {
    /// The full set an operator session holds.
    #[must_use]
    pub fn all() -> Scopes {
        Scopes {
            read: true,
            send: true,
            admin: true,
        }
    }

    /// The set an API token's scopes grant, applying the implication
    /// `admin ⊇ send ⊇ read`.
    #[must_use]
    pub fn from_token(scopes: &[TokenScope]) -> Scopes {
        let mut s = Scopes::default();
        for scope in scopes {
            match scope {
                TokenScope::Read => s.read = true,
                TokenScope::Send => {
                    s.send = true;
                    s.read = true;
                }
                TokenScope::Admin => {
                    s.admin = true;
                    s.send = true;
                    s.read = true;
                }
                _ => {}
            }
        }
        s
    }

    /// Whether this set grants `required`.
    #[must_use]
    pub fn has(&self, required: Scope) -> bool {
        match required {
            Scope::Read => self.read,
            Scope::Send => self.send,
            Scope::Admin => self.admin,
        }
    }

    /// Returns `403 insufficient_scope` when `required` is not granted.
    ///
    /// # Errors
    /// [`ApiError`] with status `403` when the credential lacks `required`.
    pub fn require(&self, required: Scope) -> Result<(), ApiError> {
        if self.has(required) {
            Ok(())
        } else {
            Err(ApiError::forbidden(
                "insufficient_scope",
                "the credential does not grant the required scope",
            ))
        }
    }
}

/// The authenticated identity attached to a request as an extension.
#[derive(Clone, Debug)]
pub struct AuthCtx {
    /// The capabilities the credential grants.
    pub scopes: Scopes,
    /// An audit label for the identity (`session` or `token:<id>`).
    pub actor: String,
}

/// An in-memory, per-peer sliding-window rate limiter.
#[derive(Clone, Debug, Default)]
pub struct RateLimiter {
    inner: Arc<Mutex<HashMap<IpAddr, Vec<Instant>>>>,
}

impl RateLimiter {
    /// A fresh limiter.
    #[must_use]
    pub fn new() -> RateLimiter {
        RateLimiter::default()
    }

    /// Whether `peer` has reached the failure cap within the window.
    #[must_use]
    pub fn over_limit(&self, peer: IpAddr) -> bool {
        let mut map = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let now = Instant::now();
        let hits = map.entry(peer).or_default();
        hits.retain(|t| now.duration_since(*t) < RATE_WINDOW);
        hits.len() >= AUTH_FAIL_MAX
    }

    /// Records one failure for `peer`.
    pub fn record(&self, peer: IpAddr) {
        let mut map = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let now = Instant::now();
        let hits = map.entry(peer).or_default();
        hits.retain(|t| now.duration_since(*t) < RATE_WINDOW);
        hits.push(now);
    }
}

/// The peer IP of a request, from the connection info the server inserts.
#[must_use]
pub fn peer_ip(req: &Request) -> IpAddr {
    req.extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0.ip())
        .unwrap_or(IpAddr::from([127, 0, 0, 1]))
}

/// The paths served without a credential (the full path, including the
/// `/api/v1` prefix).
const UNAUTHENTICATED: &[&str] = &[
    "/api/v1/health",
    "/api/v1/openapi.json",
    "/api/v1/ca.der",
    "/api/v1/ca.pem",
    "/api/v1/session",
];

/// Whether `path` is served without a credential.
///
/// The listed `/api/v1` exceptions are unauthenticated, and so is any path that
/// does not start with `/api/v1`: those are the static-asset class (the embedded
/// UI bundle and its SPA deep-link fallback, served by `crosswire-server`). The
/// Host check still runs first, so static assets are served only for a loopback
/// `Host`. Every real API route starts with `/api/v1` and is not in the list, so
/// it stays gated (for example `/api/v1/counts` → `401`).
#[must_use]
pub fn is_unauthenticated(path: &str) -> bool {
    UNAUTHENTICATED.contains(&path) || !path.starts_with("/api/v1")
}

/// Whether `host` (the `Host` header value) is an accepted loopback name.
#[must_use]
pub fn host_is_loopback(host: &str) -> bool {
    let name = host.rsplit_once(':').map_or(host, |(h, _)| h);
    name.eq_ignore_ascii_case("localhost") || name == "127.0.0.1"
}

/// Whether `host` (the `Host` header value) is the isolated preview origin:
/// the IP literals `127.0.0.1` or `[::1]`/`::1`, never `localhost` (spec §11.1).
///
/// The preview layer serves captured bodies only on this origin, so a previewed
/// page lands on a DIFFERENT browser origin than the UI (`localhost`) and can
/// never read the UI's cookies or storage. `localhost` is deliberately excluded:
/// accepting it would collapse the two origins and defeat the isolation.
#[must_use]
pub fn host_is_preview_origin(host: &str) -> bool {
    // The bare IPv6 literals carry no port to strip.
    if host == "::1" || host == "[::1]" {
        return true;
    }
    // A bracketed IPv6 host puts the optional port after `]`; every other form
    // puts it after the last `:`.
    let name = if let Some(rest) = host.strip_prefix('[') {
        rest.split_once(']').map_or(host, |(inner, _)| inner)
    } else {
        host.rsplit_once(':').map_or(host, |(h, _)| h)
    };
    name == "127.0.0.1" || name == "::1" || name == "[::1]"
}

/// The `Origin` values that count as same-origin for cookie-authenticated
/// requests on this port.
#[must_use]
pub fn allowed_origins(port: u16) -> [String; 2] {
    [
        format!("http://localhost:{port}"),
        format!("http://127.0.0.1:{port}"),
    ]
}

/// Enforces the cookie-isolation rule: a cookie-authenticated request must
/// carry `Sec-Fetch-Site: same-origin` or an `Origin` exactly equal to the
/// API origin.
fn cookie_isolation_ok(headers: &http::HeaderMap, port: u16) -> bool {
    if headers
        .get("sec-fetch-site")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("same-origin"))
    {
        return true;
    }
    let origins = allowed_origins(port);
    headers
        .get(http::header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|o| origins.iter().any(|a| a == o))
}

/// The value of the `cw_sid_<port>` cookie, if present.
#[must_use]
pub fn session_cookie(headers: &http::HeaderMap, port: u16) -> Option<&str> {
    let name = format!("cw_sid_{port}");
    let cookies = headers.get(http::header::COOKIE)?.to_str().ok()?;
    for pair in cookies.split(';') {
        let pair = pair.trim();
        if let Some((k, v)) = pair.split_once('=') {
            if k.trim() == name {
                return Some(v.trim());
            }
        }
    }
    None
}

/// The bearer token from the `Authorization` header, if present.
#[must_use]
pub fn bearer(headers: &http::HeaderMap) -> Option<&str> {
    let value = headers.get(http::header::AUTHORIZATION)?.to_str().ok()?;
    value.strip_prefix("Bearer ").map(str::trim)
}

/// Authenticates a request from its headers: bearer token first, then the
/// session cookie with its isolation check.
///
/// On success returns the granted [`AuthCtx`]. On failure returns the error
/// to send; the caller records and audits the failure.
pub fn authenticate(state: &AppState, headers: &http::HeaderMap) -> Result<AuthCtx, ApiError> {
    if let Some(token) = bearer(headers) {
        return authenticate_token(state, token);
    }

    if let Some(id) = session_cookie(headers, state.port) {
        if !cookie_isolation_ok(headers, state.port) {
            return Err(ApiError::forbidden(
                "origin_forbidden",
                "a cookie-authenticated request must be same-origin",
            ));
        }
        if state.sessions.validate_and_refresh(id, state.idle_timeout) {
            return Ok(AuthCtx {
                scopes: Scopes::all(),
                actor: "session".to_string(),
            });
        }
        return Err(ApiError::unauthorized(
            "session_invalid",
            "the session is unknown or expired",
        ));
    }

    Err(ApiError::unauthorized(
        "missing_credential",
        "no session cookie or bearer token was presented",
    ))
}

/// Authenticates a bare API-token secret (used by the bearer header path and
/// the WebSocket first-frame path).
///
/// # Errors
/// [`ApiError`] with status `401` when the token is malformed, unknown or
/// revoked, or `500` when the store lookup fails.
pub fn authenticate_token(state: &AppState, token: &str) -> Result<AuthCtx, ApiError> {
    let Some(hash) = token_hash(token) else {
        return Err(ApiError::unauthorized(
            "invalid_token",
            "the bearer token is malformed",
        ));
    };
    let record = state
        .control
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .token_lookup(&hash)
        .map_err(|e| ApiError::internal(format!("token lookup failed: {e}")))?;
    match record {
        Some(rec) if rec.revoked_at.is_none() => Ok(AuthCtx {
            scopes: Scopes::from_token(&rec.scopes),
            actor: format!("token:{}", rec.id),
        }),
        _ => Err(ApiError::unauthorized(
            "invalid_token",
            "the bearer token is unknown or revoked",
        )),
    }
}

/// The top-level gate: Host check, route allowlist, then the credential and
/// idle-refresh step for authenticated routes.
pub async fn gate(State(state): State<AppState>, mut req: Request, next: Next) -> Response {
    let path = req.uri().path().to_string();
    // The `Host` is read once; the two verdicts are computed before `req` is
    // moved so the header borrow ends here.
    let host = req
        .headers()
        .get(http::header::HOST)
        .and_then(|v| v.to_str().ok());
    let host_is_preview = host.is_some_and(host_is_preview_origin);
    let host_is_loop = host.is_some_and(host_is_loopback);

    // 0. The preview layer (§11.1) is served ONLY on the isolated
    //    `127.0.0.1`/`[::1]` origin, never on `localhost`, so a previewed page
    //    lands on a different browser origin than the UI and can never read its
    //    cookies. The capability token in the query string is the credential, so
    //    the gate runs no cookie/session check here — the handler's HMAC
    //    verification is the gate. A `/preview/` request on any other `Host`
    //    (including `localhost`) is a bare `404`: there is no such resource on
    //    that origin, so the body is never served and the probe cannot tell the
    //    origin apart from one that simply has no preview route.
    if path.starts_with("/preview/") {
        if host_is_preview {
            return next.run(req).await;
        }
        return ApiError::not_found("not_found", "not found").into_response();
    }

    // 1. Host check (every non-preview route serves loopback hosts only). The
    //    preview origin is NOT widened here: the browser scopes the session
    //    cookie to the `localhost` origin, so it never reaches `127.0.0.1`, and
    //    the API on `127.0.0.1` therefore sees no cookie and stays unauthenticated.
    if !host_is_loop {
        return ApiError::misdirected("bad_host", "the API serves loopback hosts only")
            .into_response();
    }

    // 2. Route allowlist. The WebSocket route authenticates inside the
    //    handler (it may use a first-frame bearer), so it bypasses the gate
    //    credential step here.
    if is_unauthenticated(&path) || path == "/api/v1/events" {
        return next.run(req).await;
    }

    // 3. Credential (with idle refresh for cookie sessions).
    let peer = peer_ip(&req);
    if state.auth_limiter.over_limit(peer) {
        return ApiError::rate_limited("too many authentication failures").into_response();
    }
    match authenticate(&state, req.headers()) {
        Ok(ctx) => {
            req.extensions_mut().insert(ctx);
            next.run(req).await
        }
        Err(e) => {
            state.auth_limiter.record(peer);
            let _ = state
                .store
                .audit(&peer.to_string(), "auth.fail", &path, e.code);
            e.into_response()
        }
    }
}
