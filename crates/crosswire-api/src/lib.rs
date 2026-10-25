//! REST and WebSocket API for the Crosswire sidecar.
//!
//! This crate builds the loopback HTTP/1 + WebSocket API that the web UI and
//! scripts drive. It is the security boundary of the sidecar: it issues
//! bootstrap sessions, mints and checks scoped API tokens, isolates cookies by
//! port and `Origin`, and authenticates the WebSocket.
//!
//! The entry points are [`AppState`] (everything the handlers share),
//! [`router`] (the [`axum::Router`] over that state), [`bootstrap_token`]
//! (mint the single-use bootstrap token the server delivers out of band), and
//! [`ApiConfig`] (the listener address and credential lifetimes). The server
//! that binds the loopback listener and delivers the token is `crosswire
//! serve` in the `crosswire-server` crate.
//!
//! # Security model (spec §11.2)
//!
//! - The only unauthenticated routes are `GET /api/v1/ca.der`,
//!   `GET /api/v1/ca.pem`, `POST /api/v1/session`, `GET /api/v1/openapi.json`
//!   and `GET /api/v1/health`.
//! - Every other route needs a session cookie (`cw_sid_<port>`, `HttpOnly`,
//!   `SameSite=Strict`, `Path=/api`) or an `Authorization: Bearer` API token.
//! - A cookie-authenticated request must carry `Sec-Fetch-Site: same-origin`
//!   or an `Origin` exactly equal to the API origin, or it is rejected `403`.
//! - The API is served only for the `Host`s `localhost[:port]` and
//!   `127.0.0.1[:port]`; anything else is `421`. No `Access-Control-*` header
//!   is ever emitted.

mod app;
mod auth;
mod error;
mod events;
mod extract;
mod openapi;
mod routes;
mod session;
mod tokens;

pub use app::{
    bootstrap_token, router, router_with_fallback, ApiConfig, AppState, DEFAULT_BOOTSTRAP_TTL,
    DEFAULT_IDLE_TIMEOUT,
};
pub use auth::{Scope, Scopes};
pub use error::{ApiError, ApiResult};
pub use openapi::document;
pub use session::SessionStore;
pub use tokens::{token_hash, BootstrapStore};
