//! The repeater: send a raw HTTP/1.1 request, optionally fix its framing,
//! check scope, follow redirects and record every exchange.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use bytes::{Bytes, BytesMut};
use crosswire_client::{ClientError, Session, Target, Transport, VerifyMode, WritePlan};
use crosswire_core::{Change, ChangeReason};
use crosswire_h1::{
    apply_edits, content_length_edit, crlf_edits, Parser, ParserConfig, Progress, RawMessage,
    ResponseContext, StartLine,
};
use crosswire_model::{Diff, ExchangeSink, Namespace, NamespaceKind, ScopeState, SinkError};
use crosswire_scope::{Compiled, Input, PairVerdict, Scheme};
use ulid::Ulid;

use crate::record::{record_exchange, RecordError};
use crate::redirect::Url;

/// Whether the repeater may rewrite the request before sending.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Fix bare-LF line endings and `Content-Length` before sending.
    Assisted,
    /// Send the raw bytes exactly as given.
    Raw,
}

/// How the repeater follows `3xx` redirects.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FollowRedirects {
    /// Never follow.
    Off,
    /// Follow only while the target stays in scope.
    InScope,
    /// Always follow, in or out of scope.
    Always,
}

/// One repeater send.
#[derive(Clone, Debug)]
pub struct SendRequest {
    /// The owning repeater tab.
    pub tab: Ulid,
    /// A prior exchange this send was derived from, recorded as a tag.
    pub source: Option<Ulid>,
    /// The host to dial.
    pub host: String,
    /// The port to dial.
    pub port: u16,
    /// Whether to connect over TLS.
    pub tls: bool,
    /// The SNI to send and verify against (defaults to `host`).
    pub sni: Option<String>,
    /// How the TLS certificate chain is verified.
    pub verify: VerifyMode,
    /// The raw request bytes.
    pub raw: Bytes,
    /// Whether to rewrite the request before sending.
    pub mode: Mode,
    /// How to follow redirects.
    pub follow: FollowRedirects,
    /// Whether to reuse a kept-alive connection to this destination.
    pub reuse: bool,
    /// Whether an out-of-scope send is allowed when strict mode is off.
    pub allow_out_of_scope: bool,
}

/// The result of a successful send.
#[derive(Clone, Debug)]
pub struct SendOutcome {
    /// The id of the primary recorded exchange.
    pub exchange_id: Ulid,
    /// The scope verdict for the primary exchange.
    pub scope_state: ScopeState,
    /// The edits assisted mode applied.
    pub changes: Vec<Change>,
    /// Human-readable notes about edits that were deliberately skipped, e.g.
    /// `"content-length: skipped, TE present"`.
    pub skipped: Vec<String>,
    /// Whether the connection accepted an invalid certificate chain.
    pub cert_override: bool,
    /// Time to first response byte, in milliseconds.
    pub ttfb_ms: u32,
    /// Total exchange time, in milliseconds.
    pub total_ms: u32,
    /// The ids of the recorded redirect-hop exchanges, in order.
    pub redirects: Vec<Ulid>,
}

/// Why a send failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SendError {
    /// The request is out of scope and the send was refused.
    #[error("out of scope: {host}")]
    OutOfScope {
        /// The request's `Host`.
        host: String,
    },
    /// The raw request could not be parsed (assisted mode) or recorded.
    #[error("parse: {0}")]
    Parse(String),
    /// The upstream exchange failed.
    #[error("client: {0}")]
    Client(#[source] ClientError),
    /// The exchange sink refused an exchange.
    #[error("sink: {0}")]
    Sink(#[source] SinkError),
    /// Redirects exceeded the ten-hop cap.
    #[error("redirect cap exceeded")]
    RedirectCap,
}

type AuditHook = Arc<dyn Fn(&str, &str, &str) + Send + Sync>;

/// Sends raw HTTP/1.1 requests, records them, and follows redirects.
///
/// The repeater never depends on the store: it writes through an
/// [`ExchangeSink`] and reports security-relevant actions through an audit
/// hook `(action, target, result)` so the caller can log them.
pub struct Repeater {
    transport: Arc<Transport>,
    sink: Arc<dyn ExchangeSink>,
    scope: Arc<RwLock<Compiled>>,
    strict: Arc<AtomicBool>,
    audit: AuditHook,
    cfg: ParserConfig,
    sessions: Mutex<HashMap<(String, u16, bool), Session>>,
}

impl std::fmt::Debug for Repeater {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Repeater")
            .field("strict", &self.strict.load(Ordering::Relaxed))
            .field("cfg", &self.cfg)
            .finish_non_exhaustive()
    }
}

impl Repeater {
    /// Builds a repeater. `audit` is called `(action, target, result)` for
    /// each send (`"repeater.send"`) and each out-of-scope send that proceeds
    /// (`"repeater.send.out_of_scope"`).
    #[must_use]
    pub fn new(
        transport: Arc<Transport>,
        sink: Arc<dyn ExchangeSink>,
        scope: Arc<RwLock<Compiled>>,
        strict: Arc<AtomicBool>,
        audit: AuditHook,
        cfg: ParserConfig,
    ) -> Repeater {
        Repeater {
            transport,
            sink,
            scope,
            strict,
            audit,
            cfg,
            sessions: Mutex::new(HashMap::new()),
        }
    }

    /// Sends `req`, records the exchange (and any redirect hops), and returns
    /// the outcome.
    ///
    /// # Errors
    /// [`SendError::OutOfScope`] when the request is out of scope and strict
    /// mode is on or `allow_out_of_scope` is false; [`SendError::Parse`] when
    /// assisted mode cannot parse the request or recording fails;
    /// [`SendError::Client`] on a wire error; [`SendError::Sink`] when the
    /// sink refuses an exchange; [`SendError::RedirectCap`] when redirects
    /// exceed ten hops.
    pub async fn send(&self, req: SendRequest) -> Result<SendOutcome, SendError> {
        let parsed = parse_request(&req.raw, &self.cfg);
        let path = parsed.path.clone();
        let msg_host = parsed.host.clone().unwrap_or_else(|| req.host.clone());

        // Scope check: the connection identity (dialled host/SNI) and the
        // message identity (Host header) against the same scope.
        let scheme = scheme_of(req.tls);
        let conn_host = req.sni.clone().unwrap_or_else(|| req.host.clone());
        let verdict = {
            let compiled = self.scope.read().unwrap();
            compiled.evaluate_pair(
                &Input {
                    host: &conn_host,
                    port: req.port,
                    scheme,
                    path: &path,
                },
                &Input {
                    host: &msg_host,
                    port: req.port,
                    scheme,
                    path: &path,
                },
            )
        };
        let scope_state = map_verdict(verdict);
        let target_str = format!("{}:{}", req.host, req.port);

        if is_out_of_scope(verdict) {
            let strict = self.strict.load(Ordering::Relaxed);
            if strict || !req.allow_out_of_scope {
                return Err(SendError::OutOfScope { host: msg_host });
            }
            (self.audit)("repeater.send.out_of_scope", &target_str, "sent");
        }

        // Build the bytes to send, the response context, and the edit record.
        let (write_bytes, ctx, changes, skipped) = match req.mode {
            Mode::Raw => (
                req.raw.clone(),
                ResponseContext::for_method(&parsed.method),
                Vec::new(),
                Vec::new(),
            ),
            Mode::Assisted => {
                let msg = parsed
                    .msg
                    .clone()
                    .ok_or_else(|| SendError::Parse("request head did not parse".into()))?;
                let (bytes, changes, skipped) = self.assist(&msg)?;
                (
                    bytes,
                    ResponseContext::for_method(&parsed.method),
                    changes,
                    skipped,
                )
            }
        };
        let mode_str = match req.mode {
            Mode::Assisted => "assisted",
            Mode::Raw => "raw",
        };

        // Primary exchange.
        let diffs: Vec<Diff> = changes.iter().map(Diff::from).collect();
        let rec = self
            .one_exchange(
                &req.host,
                req.port,
                req.tls,
                req.sni.as_deref(),
                req.verify,
                req.reuse,
                WritePlan::raw(write_bytes),
                ctx,
                scope_state,
                diffs,
                req.source,
                req.tab,
            )
            .await;
        let rec = match rec {
            Ok(r) => {
                (self.audit)("repeater.send", &format!("{target_str} {mode_str}"), "ok");
                r
            }
            Err(e) => {
                (self.audit)(
                    "repeater.send",
                    &format!("{target_str} {mode_str}"),
                    "error",
                );
                return Err(e);
            }
        };

        let exchange_id = rec.id;
        let cert_override = rec.cert_override;
        let ttfb_ms = rec.timing.ttfb_ms;
        let total_ms = rec.timing.total_ms;

        let redirects = self.follow_redirects(&req, &path, rec).await?;

        Ok(SendOutcome {
            exchange_id,
            scope_state,
            changes,
            skipped,
            cert_override,
            ttfb_ms,
            total_ms,
            redirects,
        })
    }

    /// Applies the assisted-mode edits: CRLF normalisation, then
    /// `Content-Length`. Returns the bytes to send, the recorded changes and
    /// any skipped-edit notes.
    fn assist(&self, msg: &RawMessage) -> Result<(Bytes, Vec<Change>, Vec<String>), SendError> {
        let mut changes = Vec::new();
        let mut skipped = Vec::new();

        let crlf = crlf_edits(msg);
        let msg = if crlf.is_empty() {
            msg.clone()
        } else {
            let e = apply_edits(msg, &crlf, &self.cfg, None, ChangeReason::ContentLength)
                .map_err(|e| SendError::Parse(e.to_string()))?;
            changes.extend(e.changes);
            e.message
        };

        let has_te = msg
            .head()
            .header(msg.bytes(), "transfer-encoding")
            .is_some();
        let msg = match content_length_edit(&msg) {
            Some(cl) => {
                let e = apply_edits(&msg, &[cl], &self.cfg, None, ChangeReason::ContentLength)
                    .map_err(|e| SendError::Parse(e.to_string()))?;
                changes.extend(e.changes);
                e.message
            }
            None => {
                if has_te {
                    skipped.push("content-length: skipped, TE present".to_string());
                }
                msg
            }
        };

        let mut out = BytesMut::from(&msg.bytes()[..]);
        if let Some(body) = msg.body() {
            out.extend_from_slice(body);
        }
        Ok((out.freeze(), changes, skipped))
    }

    /// Dials (reusing a kept-alive connection when asked), records one
    /// exchange, and returns its facts.
    #[allow(clippy::too_many_arguments)]
    async fn one_exchange(
        &self,
        host: &str,
        port: u16,
        tls: bool,
        sni: Option<&str>,
        verify: VerifyMode,
        reuse: bool,
        plan: WritePlan,
        ctx: ResponseContext,
        scope_state: ScopeState,
        diffs: Vec<Diff>,
        source: Option<Ulid>,
        tab: Ulid,
    ) -> Result<crate::record::Recorded, SendError> {
        let key = (host.to_string(), port, tls);
        let existing = if reuse {
            self.sessions
                .lock()
                .unwrap()
                .remove(&key)
                .filter(Session::is_reusable)
        } else {
            None
        };
        let mut session = match existing {
            Some(s) => s,
            None => {
                let target = build_target(host, port, tls, sni, verify);
                let conn = self
                    .transport
                    .connect(&target)
                    .await
                    .map_err(SendError::Client)?;
                Session::new(conn, self.cfg)
            }
        };

        let ns = Namespace::new(NamespaceKind::Repeater, Some(tab));
        let rec = record_exchange(
            &mut session,
            plan,
            ctx,
            Arc::clone(&self.sink),
            ns,
            scope_state,
            diffs,
            source,
        )
        .await
        .map_err(map_record_err)?;

        if reuse && session.is_reusable() {
            self.sessions.lock().unwrap().insert(key, session);
        }
        Ok(rec)
    }

    /// Follows the redirect chain starting from `primary`'s response, per the
    /// request's policy, up to ten hops.
    async fn follow_redirects(
        &self,
        req: &SendRequest,
        path: &str,
        primary: crate::record::Recorded,
    ) -> Result<Vec<Ulid>, SendError> {
        let mut redirects = Vec::new();
        let mut current = Url::new(req.tls, req.host.clone(), req.port, path.to_string());
        let mut last = primary;

        loop {
            if req.follow == FollowRedirects::Off {
                break;
            }
            if !is_redirect(last.status) {
                break;
            }
            let Some(loc) = last.location.as_deref() else {
                break;
            };
            let Some(next) = current.resolve(loc) else {
                break;
            };

            let verdict = {
                let input = Input {
                    host: &next.host,
                    port: next.port,
                    scheme: scheme_of(next.tls),
                    path: &next.path,
                };
                self.scope.read().unwrap().evaluate_pair(&input, &input)
            };
            if req.follow == FollowRedirects::InScope && is_out_of_scope(verdict) {
                break;
            }
            if redirects.len() >= 10 {
                return Err(SendError::RedirectCap);
            }

            let hop_raw = format!(
                "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
                next.path, next.host
            );
            let target_str = format!("{}:{}", next.host, next.port);
            let rec = self
                .one_exchange(
                    &next.host,
                    next.port,
                    next.tls,
                    Some(next.host.as_str()),
                    req.verify,
                    false,
                    WritePlan::raw(Bytes::from(hop_raw.into_bytes())),
                    ResponseContext::for_method(b"GET"),
                    map_verdict(verdict),
                    Vec::new(),
                    req.source,
                    req.tab,
                )
                .await;
            let rec = match rec {
                Ok(r) => {
                    (self.audit)("repeater.send", &format!("{target_str} raw"), "ok");
                    r
                }
                Err(e) => {
                    (self.audit)("repeater.send", &format!("{target_str} raw"), "error");
                    return Err(e);
                }
            };
            redirects.push(rec.id);
            current = next;
            last = rec;
        }
        Ok(redirects)
    }
}

/// What a best-effort parse of the raw request yields.
struct Parsed {
    msg: Option<RawMessage>,
    method: Bytes,
    host: Option<String>,
    path: String,
}

/// Parses `raw` as a request, tolerating failure: on a parse error the method
/// defaults to `GET`, the host is unknown and the path is `/`.
fn parse_request(raw: &Bytes, cfg: &ParserConfig) -> Parsed {
    let mut parser = Parser::request(cfg);
    let Ok(step) = parser.feed(raw) else {
        return Parsed {
            msg: None,
            method: Bytes::from_static(b"GET"),
            host: None,
            path: "/".to_string(),
        };
    };
    let Progress::Head(head) = step.event else {
        return Parsed {
            msg: None,
            method: Bytes::from_static(b"GET"),
            host: None,
            path: "/".to_string(),
        };
    };
    let head_bytes = head.bytes().clone();
    let method = match head.head().start() {
        StartLine::Request { method, .. } => head_bytes
            .get(method.range())
            .map(Bytes::copy_from_slice)
            .unwrap_or_else(|| Bytes::from_static(b"GET")),
        _ => Bytes::from_static(b"GET"),
    };
    let host = head
        .head()
        .header(&head_bytes, "host")
        .and_then(|span| head_bytes.get(span.range()))
        .map(|v| host_only(&String::from_utf8_lossy(v)));
    let path = head
        .head()
        .start_target_range()
        .and_then(|r| head_bytes.get(r))
        .map(|t| target_path(&String::from_utf8_lossy(t)))
        .unwrap_or_else(|| "/".to_string());

    let body = raw.slice(step.consumed..);
    let msg = if body.is_empty() {
        head
    } else {
        head.with_inline_body(body)
    };
    Parsed {
        msg: Some(msg),
        method,
        host,
        path,
    }
}

/// The host part of a `Host` header value, without the port.
fn host_only(value: &str) -> String {
    let v = value.trim();
    if let Some(rest) = v.strip_prefix('[') {
        // Bracketed IPv6.
        if let Some(end) = rest.find(']') {
            return rest[..end].to_string();
        }
    }
    match v.rsplit_once(':') {
        Some((host, port)) if port.chars().all(|c| c.is_ascii_digit()) && !port.is_empty() => {
            host.to_string()
        }
        _ => v.to_string(),
    }
}

/// The path of a request target (origin-form as-is; absolute-form reduced to
/// its path; anything else defaults to `/`).
fn target_path(target: &str) -> String {
    let t = target.trim();
    if t.starts_with('/') {
        return t.to_string();
    }
    if let Some(i) = t.find("://") {
        let after = &t[i + 3..];
        if let Some(j) = after.find('/') {
            return after[j..].to_string();
        }
        return "/".to_string();
    }
    "/".to_string()
}

fn scheme_of(tls: bool) -> Scheme {
    if tls {
        Scheme::Https
    } else {
        Scheme::Http
    }
}

fn build_target(host: &str, port: u16, tls: bool, sni: Option<&str>, verify: VerifyMode) -> Target {
    if tls {
        Target::tls(host, port, sni.unwrap_or(host), verify)
    } else {
        Target::plain(host, port)
    }
}

fn map_verdict(verdict: PairVerdict) -> ScopeState {
    match verdict {
        PairVerdict::In => ScopeState::In,
        PairVerdict::Out => ScopeState::Out,
        PairVerdict::Empty => ScopeState::Empty,
        PairVerdict::HostMismatch => ScopeState::HostMismatch,
    }
}

fn is_out_of_scope(verdict: PairVerdict) -> bool {
    matches!(verdict, PairVerdict::Out | PairVerdict::HostMismatch)
}

fn is_redirect(status: Option<u16>) -> bool {
    matches!(status, Some(s) if (300..=399).contains(&s))
}

fn map_record_err(e: RecordError) -> SendError {
    match e {
        RecordError::Client(c) => SendError::Client(c),
        RecordError::Sink(s) => SendError::Sink(s),
        RecordError::RequestHead(s) => SendError::Parse(s),
        RecordError::Truncated(n) => {
            SendError::Parse(format!("response exceeded the raw cap at {n} bytes"))
        }
        RecordError::Join(s) => SendError::Parse(s),
    }
}
