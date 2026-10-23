//! The HTTP/1.1 relay: one client connection to one upstream connection
//! at a time, through a request parser and a response parser, forwarding
//! exactly the bytes each parser consumed.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::{Bytes, BytesMut};
use crosswire_client::{Conn, Target};
use crosswire_core::Diagnostic;
use crosswire_h1::{
    BodyFraming, Parser, ParserConfig, Progress, RawMessage, ResponseContext, StartLine,
};
use crosswire_model::{ConnectionInfo, Diff, ExchangeState, Marker, MarkerKind, Message, Timing};
use tokio::io::AsyncWriteExt;

use crate::capture::{CaptureBuf, CaptureResult};
use crate::listener::{ConnCtx, Shared};
use crate::pump::{Probe, Pump, PumpEnd};
use crate::rewrite::{rewrite_request_head, Mode};
use crate::{front, mitm, Decision, ProxyError};

/// Most interim (1xx) responses forwarded before the final one.
const MAX_INTERIM: usize = 16;
/// How long closing the client side may take.
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
    Close,
}

/// Relays requests from `client` until either side ends the connection.
///
/// `first` is the request head already read by the listener and the bytes
/// after it (plain absolute-form path); `None` inside a MITM tunnel. The
/// upstream is connected when the first request head has been accepted
/// by the handler.
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
    let mut relay = Relay {
        shared: Arc::clone(&ctx.shared),
        peer: ctx.peer.to_string(),
        plain,
        dest,
        upstream: None,
        req_pump: Pump::new(initial),
        resp_pump: Pump::new(Bytes::new()),
        req_parser: Parser::request(&ctx.shared.cfg.limits.parse),
        resp_parser: Parser::response(
            &ctx.shared.cfg.limits.parse,
            ResponseContext::for_method(b"GET"),
        ),
    };
    let result = loop {
        match relay.exchange(&mut *client).await {
            Ok(Next::Continue) => {}
            Ok(Next::Close) => break Ok(()),
            Err(e) => break Err(e),
        }
    };
    let _ = tokio::time::timeout(CLOSE_GRACE, client.shutdown()).await;
    if let Some(mut up) = relay.upstream.take() {
        let _ = tokio::time::timeout(CLOSE_GRACE, up.io.shutdown()).await;
    }
    drop(ctx);
    result
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
    async fn exchange(&mut self, client: &mut dyn Conn) -> Result<Next, ProxyError> {
        let shared = Arc::clone(&self.shared);
        let limits = &shared.cfg.limits;
        let handler = &shared.deps.handler;

        // Request head.
        self.req_parser.reset();
        self.req_pump.arm();
        let head = match tokio::time::timeout(
            limits.idle,
            self.req_pump.head(client, &mut self.req_parser),
        )
        .await
        {
            Err(_) | Ok(Ok(None)) => return Ok(Next::Close),
            Ok(Err(e)) => return Err(e),
            Ok(Ok(Some(head))) => head,
        };
        let started = self.req_pump.first_byte().unwrap_or_else(Instant::now);
        let StartLine::Request { method, target, .. } = *head.head().start() else {
            return Err(ProxyError::Protocol("not a request".into()));
        };
        let method = head.bytes().slice(method.range());
        let mode = self.mode(&head.bytes()[target.range()])?;

        let client_info =
            ConnectionInfo::new(&self.dest.host, self.dest.port).with_client_addr(&self.peer);
        if handler.on_request_head(&head, &client_info) == Decision::Drop {
            return Ok(Next::Close);
        }
        let (rewritten, diffs) = rewrite_request_head(&head, &mode);

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
        if self.upstream.is_none() {
            self.upstream = Some(connect(&shared, &self.dest, &self.peer).await?);
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

        // Request head and body, client to upstream.
        up.io.write_all(&rewritten).await.map_err(ProxyError::Io)?;
        up.io.flush().await.map_err(ProxyError::Io)?;
        let mut req_cap = CaptureBuf::new(limits.capture_cap, &shared.budget);
        let end = self
            .req_pump
            .body(
                client,
                &mut self.req_parser,
                &mut *up.io,
                &mut req_cap,
                |d| handler.on_request_body_chunk(d),
            )
            .await?;
        if end == PumpEnd::Dropped {
            return Ok(Next::Close);
        }
        let req_body_diags = self.req_parser.body_diagnostics().to_vec();
        // Requests read ahead of this one's response. Over the limit, this
        // exchange still completes; the connection closes after it.
        let depth = 1 + queued_heads(&limits.parse, self.req_pump.pending());
        let req_written = Instant::now();

        // Response: interim heads, the final head, the body.
        let resp_ctx = ResponseContext::for_method(&method);
        self.resp_parser.reset_response(resp_ctx);
        self.resp_pump.arm();
        let mut interim = Vec::new();
        let resp_head = loop {
            let Some(h) = self
                .resp_pump
                .head(&mut *up.io, &mut self.resp_parser)
                .await?
            else {
                return Err(ProxyError::Protocol(
                    "upstream closed before responding".into(),
                ));
            };
            let status = h.head().status_code(h.bytes());
            if matches!(status, Some(100..=199)) && status != Some(101) {
                if interim.len() >= MAX_INTERIM {
                    return Err(ProxyError::Protocol("too many interim responses".into()));
                }
                client.write_all(h.bytes()).await.map_err(ProxyError::Io)?;
                client.flush().await.map_err(ProxyError::Io)?;
                interim.push(h.bytes().clone());
                self.resp_parser.reset_response(resp_ctx);
                continue;
            }
            break h;
        };
        let ttfb_ms = self
            .resp_pump
            .first_byte()
            .map_or(0, |t| ms(t.saturating_duration_since(req_written)));
        if handler.on_response_head(&resp_head, &head) == Decision::Drop {
            return Ok(Next::Close);
        }
        client
            .write_all(resp_head.bytes())
            .await
            .map_err(ProxyError::Io)?;
        client.flush().await.map_err(ProxyError::Io)?;
        let mut resp_cap = CaptureBuf::new(limits.capture_cap, &shared.budget);
        let end = self
            .resp_pump
            .body(
                &mut *up.io,
                &mut self.resp_parser,
                client,
                &mut resp_cap,
                |d| handler.on_response_body_chunk(d),
            )
            .await?;
        if end == PumpEnd::Dropped {
            return Ok(Next::Close);
        }
        let resp_body_diags = self.resp_parser.body_diagnostics().to_vec();

        let close = closes_connection(&head, &resp_head) || end == PumpEnd::Eof;
        // Bytes after a complete response belong to no exchange; the
        // connection cannot be trusted to frame the next one.
        let stray = !self.resp_pump.pending().is_empty();
        let draft = ExchangeDraft {
            req_head: head,
            req_rewritten: rewritten,
            diffs,
            req_body: req_cap.finish(),
            resp_head: Some(resp_head),
            interim,
            resp_body: resp_cap.finish(),
            timing: Timing {
                connect_ms,
                tls_ms,
                ttfb_ms,
                total_ms: ms(started.elapsed()),
            },
            info,
            state: ExchangeState::Received,
            generated: None,
            body_diagnostics: (req_body_diags, resp_body_diags),
        };
        shared.counters.exchanges.fetch_add(1, Ordering::Relaxed);
        shared.recorder.record(draft);
        if stray {
            self.extra_bytes_marker();
            self.drop_upstream();
        }
        if depth > limits.max_pipelined {
            shared.recorder.submit_marker(Marker::new(
                MarkerKind::LimitExceeded,
                format!(
                    "pipelining: {depth} requests queued on one connection from {} (max {})",
                    self.peer, limits.max_pipelined
                ),
            ));
            return Err(ProxyError::Limit("pipelining"));
        }
        Ok(if close { Next::Close } else { Next::Continue })
    }

    /// Closes the upstream connection (by dropping it) and forgets any
    /// bytes read from it.
    fn drop_upstream(&mut self) {
        self.upstream = None;
        self.resp_pump = Pump::new(Bytes::new());
    }

    fn extra_bytes_marker(&self) {
        self.shared.recorder.submit_marker(Marker::new(
            MarkerKind::Error,
            format!(
                "extra bytes after response from {}:{}",
                self.dest.host, self.dest.port
            ),
        ));
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

/// Connects to `dest`. Errors close the client for now; Task 7 answers
/// them with a 502.
async fn connect(shared: &Shared, dest: &Dest, peer: &str) -> Result<Upstream, ProxyError> {
    {
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
