//! The embedded web UI bundle and the service that serves it.
//!
//! The committed, drift-checked bundle under `assets/ui/` is embedded at compile
//! time by [`rust_embed`], so `cargo build` needs no Node toolchain. [`service`]
//! builds a [`tower::Service`] that `crosswire-server` installs as the API
//! router's fallback (see [`crosswire_api::router_with_fallback`]); every
//! non-`/api/v1` path is the static-asset class
//! ([`crosswire_api::router`]'s gate lets it through), so it reaches this service.
//!
//! Each response carries the UI Content-Security-Policy and
//! `X-Content-Type-Options: nosniff`. The CSP's `style-src` carries a fresh
//! per-response nonce, and the same nonce is injected into `index.html` as
//! `<meta name="csp-nonce" content="…">`. The SPA (Task 3+) reads that meta and
//! feeds it to CodeMirror's `cspNonce` facet, so the `<style>` elements
//! CodeMirror injects at runtime satisfy `style-src 'nonce-…'`. The main app
//! script is a same-origin ES module, covered by `default-src 'self'`; the nonce
//! exists only for those dynamically injected styles.

use std::convert::Infallible;

use axum::body::Body;
use axum::extract::Request;
use axum::http::{header, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use crosswire_api::ApiError;
use rand::RngCore;
use rust_embed::RustEmbed;

/// The embedded UI bundle (the committed copy of `ui/dist/`).
#[derive(RustEmbed)]
#[folder = "assets/ui/"]
struct Assets;

/// A tower service that serves the embedded UI bundle for a given API port.
///
/// It serves an embedded file at its own path with the right `Content-Type`; for
/// an unknown non-`/api/v1` `GET`/`HEAD` path (a client-side deep link) it serves
/// `index.html` with `200`; and for an unknown `/api/v1` path (one that passed
/// the gate but matched no route) it returns a `404` in the API's JSON error
/// shape — never HTML. Every UI response carries the CSP and `nosniff`.
///
/// `api_port` is the port the CSP's `frame-src`/`connect-src` name; it is the API
/// listener's own port (the UI is served same-origin).
pub fn service(
    api_port: u16,
) -> impl tower::Service<Request, Response = Response, Error = Infallible, Future: Send + 'static>
       + Clone
       + Send
       + Sync
       + 'static {
    tower::service_fn(move |req: Request| {
        std::future::ready(Ok::<Response, Infallible>(respond(&req, api_port)))
    })
}

/// Routes one request to an embedded file, the SPA shell, or a `404`.
fn respond(req: &Request, api_port: u16) -> Response {
    let path = req.uri().path();
    let method = req.method();
    let nonce = new_nonce();

    // An exact embedded-file hit. `index.html` is served through the SPA-shell
    // path instead, so the per-response nonce is injected into it.
    let rel = path.trim_start_matches('/');
    if !rel.is_empty() && rel != "index.html" {
        if let Some(file) = Assets::get(rel) {
            let mime = mime_guess::from_path(rel).first_or_octet_stream();
            return build(StatusCode::OK, mime.as_ref(), file.data.into_owned(), api_port, &nonce);
        }
    }

    // An `/api/v1` path reached the fallback: it matched no API route and passed
    // the gate. It must never become an HTML page.
    if path.starts_with("/api/v1") {
        return ApiError::not_found("not_found", "no such API route").into_response();
    }

    // The SPA shell: `/`, `/index.html`, or any unknown non-`/api/v1` deep link.
    if method == Method::GET || method == Method::HEAD {
        return index_response(api_port, &nonce);
    }

    // A non-`GET` request to an unknown UI path.
    ApiError::not_found("not_found", "not found").into_response()
}

/// Serves `index.html` with the per-response nonce injected as a `<meta>` tag.
fn index_response(api_port: u16, nonce: &str) -> Response {
    let Some(file) = Assets::get("index.html") else {
        return ApiError::internal("the UI bundle is missing index.html").into_response();
    };
    let html = String::from_utf8_lossy(file.data.as_ref());
    let meta = format!("<meta name=\"csp-nonce\" content=\"{nonce}\">");
    let injected = if html.contains("</head>") {
        html.replacen("</head>", &format!("  {meta}\n  </head>"), 1)
    } else {
        format!("{meta}{html}")
    };
    build(
        StatusCode::OK,
        "text/html; charset=utf-8",
        injected.into_bytes(),
        api_port,
        nonce,
    )
}

/// Builds a UI response with the `Content-Type`, `nosniff` and CSP headers.
fn build(
    status: StatusCode,
    content_type: &str,
    body: Vec<u8>,
    api_port: u16,
    nonce: &str,
) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(header::CONTENT_SECURITY_POLICY, csp(api_port, nonce))
        .body(Body::from(body))
        .expect("the UI response headers are valid")
}

/// The UI Content-Security-Policy, with a fresh per-response `nonce` in
/// `style-src`. `api_port` names the only `frame-src`/`connect-src` origins.
fn csp(api_port: u16, nonce: &str) -> String {
    format!(
        "default-src 'self'; \
         frame-src http://127.0.0.1:{api_port}; \
         connect-src 'self' ws://localhost:{api_port}; \
         object-src 'none'; base-uri 'none'; frame-ancestors 'none'; \
         style-src 'self' 'nonce-{nonce}'"
    )
}

/// A fresh 128-bit base64 nonce for one response.
fn new_nonce() -> String {
    let mut bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::STANDARD.encode(bytes)
}
