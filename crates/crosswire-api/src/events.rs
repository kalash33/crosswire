//! The `GET /events` WebSocket: streams recorder events and intercept
//! held-count changes to an authenticated subscriber.
//!
//! Authentication is one of: a session cookie (with the same-origin/`Origin`
//! check), an `Authorization: Bearer` header, or a first text frame
//! `{"bearer":"..."}` sent within 5 seconds of the upgrade.

use std::net::SocketAddr;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Query, State};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use utoipa::IntoParams;

use crate::app::AppState;
use crate::auth::{authenticate, authenticate_token, bearer, session_cookie, AuthCtx, Scope};

/// How long the server waits for a first-frame bearer token.
const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(5);

/// Query parameters for `GET /events`.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct EventsQuery {
    /// Resync from this event sequence (replayed if buffered, else a single
    /// `lagged` frame is sent first).
    pub after: Option<u64>,
}

/// The first-frame bearer envelope.
#[derive(Deserialize)]
struct FirstFrame {
    bearer: String,
}

/// Opens the event stream, authenticating before the upgrade when a cookie or
/// bearer header is present, else deferring to the first frame.
#[utoipa::path(
    get,
    path = "/api/v1/events",
    params(("after" = Option<u64>, Query, description = "Resync from this event sequence")),
    responses(
        (status = 101, description = "WebSocket upgrade"),
        (status = 401, description = "Authentication failed"),
        (status = 403, description = "Origin check failed or scope insufficient")
    )
)]
pub async fn events_ws(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Query(q): Query<EventsQuery>,
    headers: http::HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    let ip = peer.ip();
    // Authenticate up front when a credential is on the request. The gate lets
    // this route through so the first-frame path can run when none is present.
    let pre: Option<AuthCtx> = if bearer(&headers).is_some()
        || session_cookie(&headers, state.port).is_some()
    {
        if state.auth_limiter.over_limit(ip) {
            return crate::error::ApiError::rate_limited("too many authentication failures")
                .into_response();
        }
        match authenticate(&state, &headers) {
            Ok(ctx) => {
                if let Err(e) = ctx.scopes.require(Scope::Read) {
                    return e.into_response();
                }
                Some(ctx)
            }
            Err(e) => {
                state.auth_limiter.record(ip);
                let _ = state
                    .store
                    .audit(&ip.to_string(), "auth.fail", "/api/v1/events", e.code);
                return e.into_response();
            }
        }
    } else {
        None
    };

    let after = q.after;
    ws.on_upgrade(move |socket| stream(socket, state, after, pre, ip))
}

/// Drives one upgraded socket.
async fn stream(
    mut socket: WebSocket,
    state: AppState,
    after: Option<u64>,
    pre: Option<AuthCtx>,
    ip: std::net::IpAddr,
) {
    // Resolve the identity: either the pre-upgrade one, or the first frame.
    if pre.is_none() && !first_frame_auth(&mut socket, &state, ip).await {
        return;
    }

    // `after` cannot be replayed from the broadcast channel, so announce the
    // gap with a single `lagged` frame and then stream live events.
    if let Some(seq) = after {
        let frame = format!("{{\"type\":\"lagged\",\"missed\":0,\"seq\":{seq}}}");
        if socket.send(Message::Text(frame.into())).await.is_err() {
            return;
        }
    }

    let mut events = state.store.subscribe();
    let mut intercept_rx = state.intercept.subscribe();

    loop {
        tokio::select! {
            ev = events.recv() => {
                match ev {
                    Some(event) => {
                        let Ok(json) = serde_json::to_string(&event) else { continue };
                        if socket.send(Message::Text(json.into())).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                }
            }
            changed = intercept_rx.changed() => {
                if changed.is_err() {
                    // The sender is gone; stop watching intercept but keep
                    // streaming events.
                    continue;
                }
                let held = state.intercept.held().len();
                let frame = format!("{{\"type\":\"intercept\",\"held\":{held}}}");
                if socket.send(Message::Text(frame.into())).await.is_err() {
                    break;
                }
            }
            incoming = socket.recv() => {
                match incoming {
                    None | Some(Err(_)) | Some(Ok(Message::Close(_))) => break,
                    _ => {}
                }
            }
        }
    }
}

/// Waits for a first-frame `{"bearer":"..."}` and authenticates it. Returns
/// `true` when authentication (including the read scope) succeeds.
async fn first_frame_auth(socket: &mut WebSocket, state: &AppState, ip: std::net::IpAddr) -> bool {
    let frame = match tokio::time::timeout(FIRST_FRAME_TIMEOUT, socket.recv()).await {
        Ok(Some(Ok(Message::Text(text)))) => text,
        _ => {
            let _ = socket.send(Message::Close(None)).await;
            return false;
        }
    };
    let Ok(parsed) = serde_json::from_str::<FirstFrame>(frame.as_str()) else {
        let _ = socket.send(Message::Close(None)).await;
        return false;
    };
    match authenticate_token(state, &parsed.bearer) {
        Ok(ctx) if ctx.scopes.has(Scope::Read) => true,
        _ => {
            state.auth_limiter.record(ip);
            let _ = state.store.audit(
                &ip.to_string(),
                "auth.fail",
                "/api/v1/events",
                "invalid_token",
            );
            let _ = socket.send(Message::Close(None)).await;
            false
        }
    }
}
