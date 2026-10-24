//! The HTTP/1.1 relay: one client connection to one upstream connection
//! at a time, through a request parser and a response parser, forwarding
//! exactly the bytes each parser consumed.
//!
//! While a request body is relayed the upstream is read too, so interim
//! responses (`100 Continue`) reach a client that waits for them, and an
//! early final response (a 413 during an upload) ends the upload. Every
//! failure ends in a generated response (when no response bytes have been
//! sent yet), a recorded `Error` exchange, a marker, or a tunnel; see
//! [`Relay::exchange`].

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::{Bytes, BytesMut};
use crosswire_client::{ClientError, Conn, Target};
use crosswire_core::Diagnostic;
use crosswire_h1::{
    BodyFraming, ParseError, Parser, ParserConfig, Progress, RawMessage, ResolutionRule,
    ResponseContext, StartLine,
};
use crosswire_model::{
    ConnectionInfo, Diff, ExchangeState, Marker, MarkerKind, Message, Timing, TunnelReason,
};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};

use crate::capture::{CaptureBuf, CaptureResult};
use crate::generated::{
    fill, generated, BAD_GATEWAY_CONNECT, BAD_GATEWAY_TLS, BAD_REQUEST_PARSE, GATEWAY_TIMEOUT,
    REFUSED_DESTINATION,
};
use crate::listener::{self, ConnCtx, Shared};
use crate::pump::{Clock, Probe, Pump, PumpEnd, PumpFail};
use crate::record::RecordCtx;
use crate::rewrite::{rewrite_request_head, Mode};
use crate::{front, mitm, tunnel, Decision, Handler, ProxyError};

/// Most interim (1xx) responses forwarded before the final one.
const MAX_INTERIM: usize = 16;
/// How long closing a side may take.
const CLOSE_GRACE: Duration = Duration::from_secs(1);

/// Where requests on a connection go: the host and port as requested (what
/// is recorded), whether the upstream speaks TLS, and how to connect.
pub(crate) struct Dest {
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) tls: bool,
    pub(crate) target: Target,
}

impl Dest {
    /// The destination of an absolute-form request to `host:port`.
    pub(crate) fn absolute(shared: &Shared, host: &str, port: u16, tls: bool) -> Dest {
        let target = if tls {
            mitm::upstream_tls_target(shared, host, port, host)
        } else {
            Target::plain(&mitm::dial_host(shared, host), port)
        };
        Dest {
            host: host.to_string(),
            port,
            tls,
            target,
        }
    }

    fn same(&self, host: &str, port: u16, tls: bool) -> bool {
        self.host.eq_ignore_ascii_case(host) && self.port == port && self.tls == tls
    }
}

/// Everything known about one relayed exchange, handed to the recorder.
pub(crate) struct ExchangeDraft {
    /// The request head as the client sent it.
    pub(crate) req_head: RawMessage,
    /// The head as forwarded upstream.
    pub(crate) req_rewritten: Bytes,
    /// The edits from `req_head` to `req_rewritten`.
    pub(crate) diffs: Vec<Diff>,
    pub(crate) req_body: CaptureResult,
    pub(crate) resp_head: Option<RawMessage>,
    /// Interim (1xx) heads forwarded before the final response.
    pub(crate) interim: Vec<Bytes>,
    pub(crate) resp_body: CaptureResult,
    pub(crate) timing: Timing,
    pub(crate) info: ConnectionInfo,
    pub(crate) state: ExchangeState,
    /// A proxy-generated response sent instead of an upstream one.
    pub(crate) generated: Option<Message>,
    /// Body diagnostics of the request and the response.
    pub(crate) body_diagnostics: (Vec<Diagnostic>, Vec<Diagnostic>),
}

/// The request of the exchange in progress, as it will be recorded.
struct Req {
    head: RawMessage,
    rewritten: Bytes,
    diffs: Vec<Diff>,
    /// When its first byte arrived.
    started: Instant,
}

/// The open upstream connection.
struct Upstream {
    io: Box<dyn Conn>,
    /// Connection facts, recorded under the requested host.
    info: ConnectionInfo,
    conn_id: u64,
    connect_ms: u32,
    tls_ms: u32,
    /// Exchanges sent on it so far.
    uses: u32,
}

/// What to do after an exchange.
enum Next {
    Continue,
    /// Close both sides. `linger` drains the client after the half-close,
    /// so request bytes it is still sending do not turn the close into a
    /// reset that destroys the response just written.
    Close {
        linger: bool,
    },
    /// Relay the rest of both connections as opaque bytes.
    Tunnel {
        reason: TunnelReason,
        info: Box<ConnectionInfo>,
        /// Read-ahead bytes flushed before the tunnel started (to the
        /// upstream, to the client): counted as tunnel traffic.
        flushed: (u64, u64),
    },
}

/// How waiting for the final response head ended.
enum HeadOutcome {
    /// The final (non-1xx, or 101) head.
    Final(RawMessage),
    /// The upstream closed (or refused the request head) before sending
    /// any response byte.
    Closed,
    /// Reading the response head failed or timed out.
    HeadFailed(PumpFail),
    /// More than [`MAX_INTERIM`] interim responses.
    TooManyInterim,
    /// Forwarding an interim response to the client failed.
    ClientGone,
    /// The request body failed before a final head arrived.
    BodyFailed(PumpFail),
    /// The handler dropped the request body.
    Dropped,
}

/// The result of [`send_request`].
struct Sent {
    outcome: HeadOutcome,
    /// The whole request body went upstream.
    body_done: bool,
    /// When the last request byte was written.
    written: Instant,
}

/// Relays requests from `client` until either side ends the connection.
///
/// `first` is the request head already read by the listener and the bytes
/// after it (plain absolute-form path); `None` inside a CONNECT tunnel.
/// The upstream is connected when the first request head has been
/// accepted by the handler. A 101 or an undelimitable response turns the
/// rest of the connection pair into a tunnel.
pub(crate) async fn run(
    mut client: Box<dyn Conn>,
    first: Option<(RawMessage, Bytes)>,
    dest: Dest,
    ctx: ConnCtx,
) -> Result<(), ProxyError> {
    let plain = first.is_some();
    let initial = first.map_or_else(Bytes::new, |(head, leftover)| {
        let mut b = BytesMut::with_capacity(head.bytes().len() + leftover.len());
        b.extend_from_slice(head.bytes());
        b.extend_from_slice(&leftover);
        b.freeze()
    });
    let shared = Arc::clone(&ctx.shared);
    let mut relay = Relay {
        shared: Arc::clone(&shared),
        peer: ctx.peer.to_string(),
        plain,
        dest,
        upstream: None,
        req_pump: Pump::new(initial),
        resp_pump: Pump::new(Bytes::new()),
        req_parser: Parser::request(&shared.cfg.limits.parse),
        resp_parser: Parser::response(
            &shared.cfg.limits.parse,
            ResponseContext::for_method(b"GET"),
        ),
    };
    let mut first_head = true;
    let result = loop {
        match relay.exchange(&mut *client, first_head).await {
            Ok(Next::Continue) => first_head = false,
            Ok(next) => break Ok(next),
            Err(e) => break Err(e),
        }
    };
    let linger = match result {
        Ok(Next::Tunnel {
            reason,
            info,
            flushed,
        }) => {
            if let Some(up) = relay.upstream.take() {
                // Free the relay's read buffers and parsers before the
                // tunnel, which brings its own: a WebSocket may live long.
                drop(relay);
                shared.counters.tunnels.fetch_add(1, Ordering::Relaxed);
                let rec = RecordCtx {
                    recorder: shared.recorder.clone(),
                    info: *info,
                };
                let stats = tunnel::run(
                    client,
                    up.io,
                    reason,
                    shared.cfg.limits.tunnel_idle,
                    rec,
                    flushed,
                )
                .await;
                tracing::debug!(?reason, ?stats, "relay tunnel closed");
            }
            drop(ctx);
            return Ok(());
        }
        Ok(Next::Close { linger }) => linger,
        _ => false,
    };
    // The upstream first: a peer waiting for our close is not held up by
    // the client's linger.
    if let Some(mut up) = relay.upstream.take() {
        let _ = tokio::time::timeout(CLOSE_GRACE, up.io.shutdown()).await;
    }
    let _ = tokio::time::timeout(CLOSE_GRACE, client.shutdown()).await;
    if linger {
        listener::linger(&mut *client).await;
    }
    drop(ctx);
    result.map(|_| ())
}

struct Relay {
    shared: Arc<Shared>,
    peer: String,
    plain: bool,
    dest: Dest,
    upstream: Option<Upstream>,
    req_pump: Pump,
    resp_pump: Pump,
    req_parser: Parser,
    resp_parser: Parser,
}

impl Relay {
    /// Relays one request and its response.
    ///
    /// The phases and what each failure becomes:
    /// 1. Request head, within `header_read` (first) or `idle` (later):
    ///    a timeout or close ends the connection silently; a parse error
    ///    is a generated 400 and a marker.
    /// 2. Upstream connect: a failure is a generated 502 (504 for a
    ///    connect timeout) recorded as an `Error` exchange.
    /// 3. Request body and response head, concurrently: interim heads are
    ///    forwarded as they come; an early final head stops the upload. An
    ///    upstream that closes before answering is retried once on a fresh
    ///    connection for an idempotent bodiless request on a reused
    ///    connection, else a 502; no head within `idle` after the body is a
    ///    504. A client that breaks the body gets a 400; one that leaves or
    ///    stalls is recorded as a partial `Error` exchange.
    ///    A request with an invalid `Content-Length` is not delimitable:
    ///    its head is sent, it is recorded as `Error`, and the rest is an
    ///    `Undelimitable` tunnel.
    /// 4. Response head: a 101 is recorded and the rest is a `Ws` tunnel; a
    ///    2xx to a `CONNECT` is recorded and the rest is an `Undelimitable`
    ///    tunnel; a head without a valid status, like any response that
    ///    does not parse, makes the rest an `Undelimitable` tunnel.
    /// 5. Response body: an upstream that closes early, a client that
    ///    leaves, or a clock that runs out ends both sides with a partial
    ///    `Error` exchange (and a marker for the first and last).
    async fn exchange(&mut self, client: &mut dyn Conn, first: bool) -> Result<Next, ProxyError> {
        let shared = Arc::clone(&self.shared);
        let limits = &shared.cfg.limits;
        let handler = &*shared.deps.handler;

        // Request head.
        self.req_parser.reset();
        self.req_pump.arm();
        let wait = if first {
            limits.header_read
        } else {
            limits.idle
        };
        self.req_pump.set_limits(None, Some(Instant::now() + wait));
        let head = match self.req_pump.head(client, &mut self.req_parser).await {
            Ok(Some(head)) => head,
            Ok(None) => return Ok(Next::Close { linger: false }),
            Err(PumpFail::Parse(e)) => return Ok(self.bad_request(client, &e).await),
            Err(e) => {
                tracing::debug!(peer = %self.peer, error = ?e, "no request head");
                return Ok(Next::Close { linger: false });
            }
        };
        let started = self.req_pump.first_byte().unwrap_or_else(Instant::now);
        let deadline = started + limits.total;
        let StartLine::Request { method, target, .. } = *head.head().start() else {
            return Err(ProxyError::Protocol("not a request".into()));
        };
        let method = head.bytes().slice(method.range());
        let mode = self.mode(&head.bytes()[target.range()])?;

        if handler.on_request_head(&head, &self.client_info()) == Decision::Drop {
            return Ok(Next::Close { linger: false });
        }
        let (rewritten, diffs) = rewrite_request_head(&head, &mode);
        let bodiless = matches!(
            resolved(head.head().framing()),
            BodyFraming::None | BodyFraming::ContentLength(0)
        );
        // An invalid Content-Length leaves the body length unknown: what
        // follows the head cannot be framed (RFC 9112 section 6.3 rule 5).
        let undelimitable_req = matches!(
            head.head().framing(),
            BodyFraming::Ambiguous {
                rule: ResolutionRule::InvalidContentLength,
                ..
            }
        );
        let req = Req {
            head,
            rewritten,
            diffs,
            started,
        };

        // Upstream. A kept-alive connection that has since closed, or that
        // sent bytes nobody asked for, is replaced.
        if let Some(up) = self.upstream.as_mut() {
            match self.resp_pump.probe(&mut *up.io).await {
                Probe::Idle => {}
                Probe::Closed => self.drop_upstream(),
                Probe::Stray => {
                    self.extra_bytes_marker();
                    self.drop_upstream();
                }
            }
        }

        // Request out, final response head in; retried once on a fresh
        // connection when a reused one closed before answering.
        let resp_ctx = ResponseContext::for_method(&method);
        let mut req_cap = CaptureBuf::new(limits.capture_cap, &shared.budget);
        let mut interim = Vec::new();
        let mut body_done = false;
        let mut retried = false;
        let (resp_head, info, timing) = loop {
            if self.upstream.is_none() {
                match connect(&shared, &self.dest, &self.peer).await {
                    Ok(up) => self.upstream = Some(up),
                    Err(e) => {
                        let (status, reason, body) = connect_failure(&self.dest, &e);
                        let draft = self.draft(req, req_cap, self.client_info(), Timing::default());
                        return Ok(self.respond(client, draft, status, reason, &body).await);
                    }
                }
            }
            let up = self.upstream.as_mut().expect("connected above");
            let reused = up.uses > 0;
            up.uses += 1;
            let info = up.info.clone().with_conn(up.conn_id, reused);
            let (connect_ms, tls_ms) = if reused {
                (0, 0)
            } else {
                (up.connect_ms, up.tls_ms)
            };
            if undelimitable_req {
                let timing = Timing {
                    connect_ms,
                    tls_ms,
                    ..Timing::default()
                };
                return Ok(self
                    .undelimitable_request(client, req, req_cap, info, timing, deadline)
                    .await);
            }
            self.resp_parser.reset_response(resp_ctx);
            self.resp_pump.arm();
            let sent = send_request(
                client,
                &mut *up.io,
                &req.rewritten,
                body_done,
                (&mut self.req_pump, &mut self.req_parser, &mut req_cap),
                (
                    &mut self.resp_pump,
                    &mut self.resp_parser,
                    &mut interim,
                    resp_ctx,
                ),
                handler,
                (limits.idle, deadline),
            )
            .await;
            body_done |= sent.body_done;
            let ttfb_ms = self
                .resp_pump
                .first_byte()
                .map_or(0, |t| ms(t.saturating_duration_since(sent.written)));
            let timing = Timing {
                connect_ms,
                tls_ms,
                ttfb_ms,
                total_ms: 0,
            };
            match sent.outcome {
                HeadOutcome::Final(h) => break (h, info, timing),
                HeadOutcome::Closed
                    if reused
                        && !retried
                        && bodiless
                        && interim.is_empty()
                        && idempotent(&method) =>
                {
                    tracing::debug!(host = %self.dest.host, "reused upstream closed; retrying once");
                    retried = true;
                    self.drop_upstream();
                }
                outcome => {
                    if !body_done {
                        req_cap.cut();
                    }
                    return Ok(self
                        .no_response(client, outcome, req, req_cap, interim, info, timing)
                        .await);
                }
            }
        };
        if !body_done {
            // The final response came first: the rest of the upload is not
            // relayed, and the connection closes after this exchange.
            req_cap.cut();
        }

        // Response head.
        if handler.on_response_head(&resp_head, &req.head) == Decision::Drop {
            return Ok(Next::Close { linger: false });
        }
        let status = resp_head.head().status_code(resp_head.bytes());
        self.resp_pump.set_limits(Some(limits.idle), Some(deadline));
        if status.is_none() {
            // No status code: nothing frames what follows.
            let mut raw = BytesMut::from(resp_head.bytes().as_ref());
            raw.extend_from_slice(&self.resp_pump.take_pending());
            let resp_cap = CaptureBuf::new(limits.capture_cap, &shared.budget);
            // Recorded once: the head as the response head, only what
            // follows it as the (cut) body.
            let head_len = resp_head.bytes().len();
            let mut draft = self.draft(req, req_cap, info.clone(), timing);
            draft.resp_head = Some(resp_head);
            draft.interim = interim;
            return Ok(self
                .undelimitable(
                    client,
                    (raw.freeze(), head_len),
                    draft,
                    resp_cap,
                    "invalid status line",
                    info,
                )
                .await);
        }
        if let Err(e) = self.resp_pump.send(client, resp_head.bytes()).await {
            tracing::debug!(peer = %self.peer, error = ?e, "client gone before the response head");
            let mut draft = self.draft(req, req_cap, info, timing);
            draft.resp_head = Some(resp_head);
            draft.interim = interim;
            draft.state = ExchangeState::Error;
            self.submit(draft);
            return Ok(Next::Close { linger: false });
        }
        // A 101, or a 2xx answer to a CONNECT sent inside a tunnel: no HTTP
        // follows on this connection pair.
        if status == Some(101)
            || matches!(resolved(resp_head.head().framing()), BodyFraming::Upgrade)
        {
            let reason = if status == Some(101) {
                TunnelReason::Ws
            } else {
                TunnelReason::Undelimitable
            };
            let mut draft = self.draft(req, req_cap, info.clone(), timing);
            draft.resp_head = Some(resp_head);
            draft.interim = interim;
            self.submit(draft);
            return Ok(self
                .hand_to_tunnel(client, Bytes::new(), reason, info)
                .await);
        }

        // Response body.
        let mut resp_cap = CaptureBuf::new(limits.capture_cap, &shared.budget);
        let up = self.upstream.as_mut().expect("connected above");
        let end = self
            .resp_pump
            .body(
                &mut *up.io,
                &mut self.resp_parser,
                client,
                &mut resp_cap,
                |d| handler.on_response_body_chunk(d),
            )
            .await;
        let resp_body_diags = self.resp_parser.body_diagnostics().to_vec();
        let close_resp = closes_connection(&req.head, &resp_head);
        let mut draft = self.draft(req, req_cap, info.clone(), timing);
        draft.resp_head = Some(resp_head);
        draft.interim = interim;
        draft.body_diagnostics.1 = resp_body_diags;
        let end = match end {
            Ok(PumpEnd::Dropped) => return Ok(Next::Close { linger: false }),
            Ok(end) => end,
            Err(PumpFail::Parse(e)) => {
                let raw = self.resp_pump.take_pending();
                return Ok(self
                    .undelimitable(client, (raw, 0), draft, resp_cap, &e.to_string(), info)
                    .await);
            }
            Err(fail) => {
                resp_cap.cut();
                draft.resp_body = resp_cap.finish();
                draft.state = ExchangeState::Error;
                self.submit(draft);
                self.response_body_failed(&fail);
                return Ok(Next::Close { linger: false });
            }
        };
        draft.resp_body = resp_cap.finish();

        let close = close_resp || end == PumpEnd::Eof || !body_done;
        // Bytes after a complete response belong to no exchange; the
        // connection cannot be trusted to frame the next one.
        let stray = !self.resp_pump.pending().is_empty();
        // Requests read ahead of this one's response. Over the limit, this
        // exchange still completes; the connection closes after it.
        let depth = 1 + queued_heads(&limits.parse, self.req_pump.pending());
        self.submit(draft);
        if stray {
            self.extra_bytes_marker();
            self.drop_upstream();
        }
        if body_done && depth > limits.max_pipelined {
            shared.recorder.submit_marker(Marker::new(
                MarkerKind::LimitExceeded,
                format!(
                    "pipelining: {depth} requests queued on one connection from {} (max {})",
                    self.peer, limits.max_pipelined
                ),
            ));
            return Err(ProxyError::Limit("pipelining"));
        }
        Ok(if close {
            Next::Close { linger: !body_done }
        } else {
            Next::Continue
        })
    }

    /// A draft of the exchange so far: the request with its body capture
    /// and body diagnostics, no response, state `Received`, total time
    /// up to now.
    fn draft(
        &self,
        req: Req,
        req_cap: CaptureBuf,
        info: ConnectionInfo,
        mut timing: Timing,
    ) -> ExchangeDraft {
        timing.total_ms = ms(req.started.elapsed());
        ExchangeDraft {
            req_head: req.head,
            req_rewritten: req.rewritten,
            diffs: req.diffs,
            req_body: req_cap.finish(),
            resp_head: None,
            interim: Vec::new(),
            resp_body: CaptureResult::empty(),
            timing,
            info,
            state: ExchangeState::Received,
            generated: None,
            body_diagnostics: (self.req_parser.body_diagnostics().to_vec(), Vec::new()),
        }
    }

    /// Counts and records `draft`.
    fn submit(&self, draft: ExchangeDraft) {
        self.shared
            .counters
            .exchanges
            .fetch_add(1, Ordering::Relaxed);
        self.shared.recorder.record(draft);
    }

    /// Sends a generated response, records it as the response of `draft`
    /// (state `Error`), and closes.
    async fn respond(
        &mut self,
        client: &mut dyn Conn,
        mut draft: ExchangeDraft,
        status: u16,
        reason: &str,
        body: &str,
    ) -> Next {
        let (bytes, msg) = generated(status, reason, body, draft.info.clone());
        self.write_generated(client, &bytes).await;
        draft.generated = Some(msg);
        draft.state = ExchangeState::Error;
        self.submit(draft);
        Next::Close { linger: true }
    }

    async fn write_generated(&mut self, client: &mut dyn Conn, bytes: &[u8]) {
        let write = async {
            client.write_all(bytes).await?;
            client.flush().await
        };
        match tokio::time::timeout(self.shared.cfg.limits.idle, write).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                tracing::debug!(peer = %self.peer, error = %e, "generated response not delivered");
            }
            Err(_) => tracing::debug!(peer = %self.peer, "generated response timed out"),
        }
    }

    /// A request head that does not parse: a generated 400 (not recorded:
    /// there is no request to record) and a marker.
    async fn bad_request(&mut self, client: &mut dyn Conn, e: &ParseError) -> Next {
        self.parse_marker(e);
        let body = fill(BAD_REQUEST_PARSE, &[("code", &e.to_string())]);
        let (bytes, _) = generated(400, "Bad Request", &body, self.client_info());
        self.write_generated(client, &bytes).await;
        Next::Close { linger: true }
    }

    /// `LimitExceeded` for a request over a parser limit, `Error` for any
    /// other parse failure.
    fn parse_marker(&self, e: &ParseError) {
        let kind = if matches!(
            e,
            ParseError::HeadTooLarge
                | ParseError::TooManyHeaders
                | ParseError::LineTooLong
                | ParseError::TooManyChunks
                | ParseError::TrailerTooLarge
        ) {
            MarkerKind::LimitExceeded
        } else {
            MarkerKind::Error
        };
        self.shared.recorder.submit_marker(Marker::new(
            kind,
            format!("request from {} could not be parsed: {e}", self.peer),
        ));
    }

    fn marker(&self, kind: MarkerKind, detail: String) {
        self.shared
            .recorder
            .submit_marker(Marker::new(kind, detail));
    }

    /// The exchange ended before a final response head was relayed.
    #[allow(
        clippy::too_many_arguments,
        reason = "everything recorded about the exchange so far"
    )]
    async fn no_response(
        &mut self,
        client: &mut dyn Conn,
        outcome: HeadOutcome,
        req: Req,
        req_cap: CaptureBuf,
        interim: Vec<Bytes>,
        info: ConnectionInfo,
        timing: Timing,
    ) -> Next {
        let (host, port) = (self.dest.host.clone(), self.dest.port.to_string());
        let gateway = |reason: &str| {
            fill(
                BAD_GATEWAY_CONNECT,
                &[("host", &host), ("port", &port), ("reason", reason)],
            )
        };
        let mut draft = self.draft(req, req_cap, info.clone(), timing);
        draft.interim = interim;
        match outcome {
            HeadOutcome::Final(_) => unreachable!("handled by the caller"),
            HeadOutcome::Dropped => Next::Close { linger: false },
            HeadOutcome::Closed => {
                let body = gateway("the connection closed before a response");
                self.respond(client, draft, 502, "Bad Gateway", &body).await
            }
            HeadOutcome::TooManyInterim => {
                let body = gateway("too many interim responses");
                self.respond(client, draft, 502, "Bad Gateway", &body).await
            }
            HeadOutcome::HeadFailed(PumpFail::Timeout(_)) => {
                let body = fill(
                    GATEWAY_TIMEOUT,
                    &[("host", &host), ("port", &port), ("phase", "response head")],
                );
                self.respond(client, draft, 504, "Gateway Timeout", &body)
                    .await
            }
            HeadOutcome::HeadFailed(PumpFail::Parse(e)) => {
                let mut raw = BytesMut::from(self.resp_pump.head_seen());
                raw.extend_from_slice(&self.resp_pump.take_pending());
                let resp_cap =
                    CaptureBuf::new(self.shared.cfg.limits.capture_cap, &self.shared.budget);
                // No head parsed, so there is no response to record; the
                // bytes are counted by the tunnel.
                let raw = raw.freeze();
                let n = raw.len();
                self.undelimitable(client, (raw, n), draft, resp_cap, &e.to_string(), info)
                    .await
            }
            HeadOutcome::HeadFailed(PumpFail::Eof(_)) => {
                let body = gateway("the connection closed in the middle of the response head");
                self.respond(client, draft, 502, "Bad Gateway", &body).await
            }
            HeadOutcome::HeadFailed(PumpFail::Read(e)) => {
                let body = gateway(&e.to_string());
                self.respond(client, draft, 502, "Bad Gateway", &body).await
            }
            HeadOutcome::ClientGone | HeadOutcome::HeadFailed(PumpFail::Write(_)) => {
                draft.state = ExchangeState::Error;
                self.submit(draft);
                Next::Close { linger: false }
            }
            HeadOutcome::BodyFailed(PumpFail::Parse(e)) => {
                self.parse_marker(&e);
                let body = fill(BAD_REQUEST_PARSE, &[("code", &e.to_string())]);
                self.respond(client, draft, 400, "Bad Request", &body).await
            }
            HeadOutcome::BodyFailed(PumpFail::Timeout(clock)) => {
                self.marker(
                    MarkerKind::Error,
                    format!("timeout: {} {host}:{port}", phase("request body", clock)),
                );
                draft.state = ExchangeState::Error;
                self.submit(draft);
                Next::Close { linger: false }
            }
            HeadOutcome::BodyFailed(fail) => {
                tracing::debug!(peer = %self.peer, error = ?fail, "client left mid-request-body");
                draft.state = ExchangeState::Error;
                self.submit(draft);
                Next::Close { linger: false }
            }
        }
    }

    /// The response body did not complete: a marker when the upstream
    /// closed or a clock ran out (a client that left needs none).
    fn response_body_failed(&self, fail: &PumpFail) {
        let (host, port) = (&self.dest.host, self.dest.port);
        match fail {
            PumpFail::Eof(e) => self.marker(
                MarkerKind::Error,
                format!("upstream {host}:{port} closed mid-body: {e}"),
            ),
            PumpFail::Read(e) => self.marker(
                MarkerKind::Error,
                format!(
                    "upstream {host}:{port} failed mid-body: {}",
                    crate::generated::cap(&e.to_string())
                ),
            ),
            PumpFail::Timeout(clock) => self.marker(
                MarkerKind::Error,
                format!("timeout: {} {host}:{port}", phase("response body", *clock)),
            ),
            PumpFail::Write(e) => {
                tracing::debug!(peer = %self.peer, error = %e, "client left mid-response");
            }
            PumpFail::Parse(_) => {}
        }
    }

    /// The response cannot be framed: `raw` (bytes read but not yet
    /// forwarded) goes to the client, the exchange is recorded as `Error`
    /// with `raw` after its first `recorded_elsewhere` bytes (the head,
    /// recorded as the head) as the cut response body, and the rest of the
    /// connection pair is an `Undelimitable` tunnel.
    async fn undelimitable(
        &mut self,
        client: &mut dyn Conn,
        (raw, recorded_elsewhere): (Bytes, usize),
        mut draft: ExchangeDraft,
        mut resp_cap: CaptureBuf,
        code: &str,
        info: ConnectionInfo,
    ) -> Next {
        resp_cap.push(&raw[recorded_elsewhere.min(raw.len())..]);
        resp_cap.cut();
        draft.resp_body = resp_cap.finish();
        draft.state = ExchangeState::Error;
        self.submit(draft);
        self.marker(
            MarkerKind::Error,
            format!(
                "undelimitable response from {}:{}: {}",
                self.dest.host,
                self.dest.port,
                crate::generated::cap(code)
            ),
        );
        self.hand_to_tunnel(client, raw, TunnelReason::Undelimitable, info)
            .await
    }

    /// A request whose body cannot be delimited (an invalid
    /// `Content-Length`): the head goes upstream, the exchange is recorded
    /// as `Error` with no response, and the rest of the connection pair,
    /// starting with the bytes read after the head, is an `Undelimitable`
    /// tunnel.
    async fn undelimitable_request(
        &mut self,
        client: &mut dyn Conn,
        req: Req,
        mut req_cap: CaptureBuf,
        info: ConnectionInfo,
        timing: Timing,
        deadline: Instant,
    ) -> Next {
        let idle = self.shared.cfg.limits.idle;
        self.req_pump.set_limits(Some(idle), Some(deadline));
        let sent = match self.upstream.as_mut() {
            Some(up) => self
                .req_pump
                .send(&mut *up.io, &req.rewritten)
                .await
                .is_ok(),
            None => false,
        };
        req_cap.cut();
        let mut draft = self.draft(req, req_cap, info.clone(), timing);
        draft.state = ExchangeState::Error;
        self.submit(draft);
        if !sent {
            return Next::Close { linger: false };
        }
        self.marker(
            MarkerKind::Error,
            format!(
                "undelimitable request to {}:{}: invalid Content-Length",
                self.dest.host, self.dest.port
            ),
        );
        self.hand_to_tunnel(client, Bytes::new(), TunnelReason::Undelimitable, info)
            .await
    }

    /// Forwards what each side has read ahead (`to_client` first, then the
    /// response pump's pending bytes; the request pump's pending bytes to
    /// the upstream) and hands both connections to a tunnel.
    async fn hand_to_tunnel(
        &mut self,
        client: &mut dyn Conn,
        to_client: Bytes,
        reason: TunnelReason,
        info: ConnectionInfo,
    ) -> Next {
        let to_client = [to_client, self.resp_pump.take_pending()];
        let to_upstream = self.req_pump.take_pending();
        let Some(up) = self.upstream.as_mut() else {
            return Next::Close { linger: false };
        };
        for bytes in &to_client {
            if !bytes.is_empty() && self.resp_pump.send(client, bytes).await.is_err() {
                return Next::Close { linger: false };
            }
        }
        if !to_upstream.is_empty() && self.req_pump.send(&mut *up.io, &to_upstream).await.is_err() {
            return Next::Close { linger: false };
        }
        let flushed_out: usize = to_client.iter().map(Bytes::len).sum();
        Next::Tunnel {
            reason,
            info: Box::new(info),
            flushed: (to_upstream.len() as u64, flushed_out as u64),
        }
    }

    /// Closes the upstream connection (by dropping it) and forgets any
    /// bytes read from it.
    fn drop_upstream(&mut self) {
        self.upstream = None;
        self.resp_pump = Pump::new(Bytes::new());
    }

    fn extra_bytes_marker(&self) {
        self.marker(
            MarkerKind::Error,
            format!(
                "extra bytes after response from {}:{}",
                self.dest.host, self.dest.port
            ),
        );
    }

    /// The connection record before an upstream exists.
    fn client_info(&self) -> ConnectionInfo {
        ConnectionInfo::new(&self.dest.host, self.dest.port).with_client_addr(&self.peer)
    }

    /// How to rewrite a request with `target`, switching the destination
    /// when a plain-path request names another origin.
    fn mode(&mut self, target: &[u8]) -> Result<Mode, ProxyError> {
        if !self.plain || target.starts_with(b"/") || target == b"*" {
            return Ok(Mode::Tunnel);
        }
        let abs = std::str::from_utf8(target)
            .ok()
            .and_then(front::absolute_parts)
            .ok_or_else(|| ProxyError::Protocol("invalid absolute-form target".into()))?;
        if !self.dest.same(&abs.host, abs.port, abs.tls) {
            // A keep-alive client may send the next request to another
            // origin: the old upstream connection cannot carry it.
            self.drop_upstream();
            self.dest = Dest::absolute(&self.shared, &abs.host, abs.port, abs.tls);
        }
        Ok(Mode::Plain {
            host: abs.authority,
        })
    }
}

/// `"{what}"` for the idle clock, `"total"` for the deadline.
fn phase(what: &'static str, clock: Clock) -> &'static str {
    match clock {
        Clock::Idle => what,
        Clock::Total => "total",
    }
}

/// Errors that mean the peer dropped the connection.
fn is_reset(e: &std::io::Error) -> bool {
    use std::io::ErrorKind::{BrokenPipe, ConnectionAborted, ConnectionReset, UnexpectedEof};
    matches!(
        e.kind(),
        ConnectionReset | ConnectionAborted | BrokenPipe | UnexpectedEof
    )
}

/// Methods safe to send again after the upstream dropped them unanswered.
fn idempotent(method: &[u8]) -> bool {
    matches!(method, b"GET" | b"HEAD" | b"OPTIONS" | b"TRACE")
}

/// The generated response for a failed upstream connect: status, reason
/// phrase and body.
fn connect_failure(dest: &Dest, e: &ClientError) -> (u16, &'static str, String) {
    let port = dest.port.to_string();
    let at = [("host", dest.host.as_str()), ("port", port.as_str())];
    let with = |template: &str, key: &str, value: &str| {
        let mut pairs = at.to_vec();
        pairs.push((key, value));
        fill(template, &pairs)
    };
    match e {
        ClientError::Refused(r) => (
            502,
            "Bad Gateway",
            with(REFUSED_DESTINATION, "refusal", &r.to_string()),
        ),
        ClientError::Tls(reason) => (502, "Bad Gateway", with(BAD_GATEWAY_TLS, "reason", reason)),
        ClientError::Timeout => (
            504,
            "Gateway Timeout",
            with(GATEWAY_TIMEOUT, "phase", "connect"),
        ),
        other => (
            502,
            "Bad Gateway",
            with(BAD_GATEWAY_CONNECT, "reason", &other.to_string()),
        ),
    }
}

/// Writes the request head upstream, then relays the request body while
/// reading the response: interim heads are forwarded to the client as
/// they arrive, and a final head ends the wait (and the upload, if it is
/// still running). Once the body is done, the head must arrive within
/// `idle`. `body_done` skips the body (a retry of a bodiless request).
#[allow(clippy::too_many_arguments, reason = "both directions of one exchange")]
async fn send_request(
    client: &mut dyn Conn,
    up: &mut dyn Conn,
    rewritten: &[u8],
    body_done: bool,
    (req_pump, req_parser, req_cap): (&mut Pump, &mut Parser, &mut CaptureBuf),
    (resp_pump, resp_parser, interim, resp_ctx): (
        &mut Pump,
        &mut Parser,
        &mut Vec<Bytes>,
        ResponseContext,
    ),
    handler: &dyn Handler,
    (idle, deadline): (Duration, Instant),
) -> Sent {
    let (mut cr, mut cw) = tokio::io::split(client);
    let (mut ur, mut uw) = tokio::io::split(up);
    req_pump.set_limits(Some(idle), Some(deadline));
    // No idle limit while the body uploads: the upstream may rightly say
    // nothing until it has all of it.
    resp_pump.set_limits(None, Some(deadline));
    let mut written = Instant::now();
    if let Err(e) = req_pump.send(&mut uw, rewritten).await {
        let outcome = match e {
            PumpFail::Timeout(_) => HeadOutcome::HeadFailed(e),
            _ => HeadOutcome::Closed,
        };
        return Sent {
            outcome,
            body_done,
            written,
        };
    }
    written = Instant::now();
    let body = async {
        if body_done {
            return Ok(PumpEnd::Done);
        }
        req_pump
            .body(&mut cr, req_parser, &mut uw, req_cap, |d| {
                handler.on_request_body_chunk(d)
            })
            .await
    };
    let heads = final_head(resp_pump, &mut ur, resp_parser, &mut cw, interim, resp_ctx);
    tokio::pin!(body, heads);
    let mut done = body_done;
    tokio::select! {
        // The body first: when both are ready, a body that just completed
        // is not mistaken for one cut short by an early response.
        biased;
        end = &mut body => match end {
            Ok(PumpEnd::Dropped) => {
                return Sent { outcome: HeadOutcome::Dropped, body_done: false, written };
            }
            Ok(_) => {
                done = true;
                written = Instant::now();
            }
            // The upstream stopped taking the body; it may still answer.
            Err(PumpFail::Write(e)) => {
                tracing::debug!(error = %e, "upstream stopped reading the request body");
            }
            Err(e) => {
                return Sent { outcome: HeadOutcome::BodyFailed(e), body_done: false, written };
            }
        },
        outcome = &mut heads => {
            return Sent { outcome, body_done: false, written };
        }
    }
    let outcome = match tokio::time::timeout(idle, &mut heads).await {
        Ok(outcome) => outcome,
        Err(_) => HeadOutcome::HeadFailed(PumpFail::Timeout(Clock::Idle)),
    };
    Sent {
        outcome,
        body_done: done,
        written,
    }
}

/// Reads response heads, forwarding interim (1xx other than 101) heads to
/// the client and collecting them, until the final head.
async fn final_head<R, W>(
    pump: &mut Pump,
    from: &mut R,
    parser: &mut Parser,
    client: &mut W,
    interim: &mut Vec<Bytes>,
    ctx: ResponseContext,
) -> HeadOutcome
where
    R: AsyncRead + Unpin + ?Sized,
    W: AsyncWrite + Unpin + ?Sized,
{
    loop {
        let head = match pump.head(from, parser).await {
            Ok(Some(head)) => head,
            Ok(None) => return HeadOutcome::Closed,
            // A reset before any response byte is a close by another name
            // (a kept-alive peer giving up), so it may be retried too.
            Err(PumpFail::Read(e)) if pump.first_byte().is_none() && is_reset(&e) => {
                return HeadOutcome::Closed;
            }
            Err(e) => return HeadOutcome::HeadFailed(e),
        };
        let status = head.head().status_code(head.bytes());
        if !matches!(status, Some(100..=199)) || status == Some(101) {
            return HeadOutcome::Final(head);
        }
        if interim.len() >= MAX_INTERIM {
            return HeadOutcome::TooManyInterim;
        }
        if pump.send(client, head.bytes()).await.is_err() {
            return HeadOutcome::ClientGone;
        }
        interim.push(head.bytes().clone());
        parser.reset_response(ctx);
    }
}

/// Connects to `dest`.
async fn connect(shared: &Shared, dest: &Dest, peer: &str) -> Result<Upstream, ClientError> {
    let conn = shared.deps.transport.connect(&dest.target).await?;
    let (io, up, connect_ms, tls_ms) = conn.into_parts();
    let mut info = ConnectionInfo::new(&dest.host, dest.port)
        .with_client_addr(peer)
        .with_chain_result(up.upstream_chain_result().clone())
        .with_cert_override(up.cert_override());
    if let Some(p) = up.peer_addr() {
        info = info.with_peer_addr(p);
    }
    if let Some(s) = up.sni() {
        info = info.with_sni(s);
    }
    if let Some(a) = up.alpn() {
        info = info.with_alpn(a);
    }
    if let (Some(v), Some(c)) = (up.tls_version(), up.cipher()) {
        info = info.with_tls(v, c);
    }
    Ok(Upstream {
        io,
        info,
        conn_id: up.conn_id(),
        connect_ms,
        tls_ms,
        uses: 0,
    })
}

fn ms(d: Duration) -> u32 {
    u32::try_from(d.as_millis()).unwrap_or(u32::MAX)
}

/// The `Connection` tokens and version of a head: (close, keep-alive,
/// HTTP/1.0).
fn connection_semantics(msg: &RawMessage) -> (bool, bool, bool) {
    let buf = msg.bytes();
    let view = msg.head();
    let version = match view.start() {
        StartLine::Request { version, .. } | StartLine::Response { version, .. } => {
            buf.get(version.range())
        }
        _ => None,
    };
    let http10 = version.is_some_and(|v| v.eq_ignore_ascii_case(b"HTTP/1.0"));
    let (mut close, mut keep_alive) = (false, false);
    for h in view.headers() {
        let name = buf.get(h.name().range()).unwrap_or_default();
        if !trim(name).eq_ignore_ascii_case(b"connection") {
            continue;
        }
        let value = buf.get(h.value().range()).unwrap_or_default();
        for token in value.split(|&b| b == b',') {
            close |= trim(token).eq_ignore_ascii_case(b"close");
            keep_alive |= trim(token).eq_ignore_ascii_case(b"keep-alive");
        }
    }
    (close, keep_alive, http10)
}

fn trim(b: &[u8]) -> &[u8] {
    let start = b
        .iter()
        .position(|&c| c != b' ' && c != b'\t')
        .unwrap_or(b.len());
    let end = b
        .iter()
        .rposition(|&c| c != b' ' && c != b'\t')
        .map_or(start, |i| i + 1);
    &b[start..end]
}

/// Whether the connection ends after this exchange: `Connection: close`
/// on either side, HTTP/1.0 without keep-alive on either side, a body
/// delimited by the close, or a switch of protocols (relayed as a tunnel
/// in Task 7; closed for now).
fn closes_connection(req: &RawMessage, resp: &RawMessage) -> bool {
    let (req_close, req_ka, req_10) = connection_semantics(req);
    let (resp_close, resp_ka, resp_10) = connection_semantics(resp);
    let status = resp.head().status_code(resp.bytes());
    req_close
        || resp_close
        || (req_10 && !req_ka)
        || (resp_10 && !resp_ka)
        || status == Some(101)
        || matches!(
            resolved(resp.head().framing()),
            BodyFraming::UntilClose | BodyFraming::Upgrade
        )
}

fn resolved(f: &BodyFraming) -> &BodyFraming {
    match f {
        BodyFraming::Ambiguous { resolved, .. } => resolved,
        other => other,
    }
}

/// How many complete request heads `bytes` holds (requests read ahead of
/// their responses).
fn queued_heads(cfg: &ParserConfig, bytes: &[u8]) -> usize {
    let mut parser = Parser::request(cfg);
    let (mut off, mut heads) = (0, 0);
    while off < bytes.len() {
        let Ok(step) = parser.feed(&bytes[off..]) else {
            break;
        };
        off += step.consumed;
        match step.event {
            Progress::Head(_) => heads += 1,
            Progress::Done => parser.reset(),
            Progress::NeedMore => break,
            _ => {}
        }
    }
    heads
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_complete_heads_skipping_bodies() {
        let cfg = ParserConfig::default();
        let two =
            b"POST /a HTTP/1.1\r\nContent-Length: 3\r\n\r\nabcGET /b HTTP/1.1\r\n\r\nGET /c HT";
        assert_eq!(queued_heads(&cfg, two), 2);
        assert_eq!(queued_heads(&cfg, b""), 0);
    }
}
