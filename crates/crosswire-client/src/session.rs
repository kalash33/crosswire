//! An HTTP/1 session over one upstream connection.

use std::time::{Duration, Instant};

use bytes::BytesMut;
use crosswire_h1::{Parser, ParserConfig, Progress, ResponseContext};
use crosswire_model::ConnectionInfo;
use tokio::io::AsyncWriteExt;

use crate::error::ClientError;
use crate::stream::{close_semantics, ResponseStream};
use crate::transport::Connection;
use crate::write_plan::WritePlan;

/// Idle time allowed for one read or write before [`ClientError::Timeout`].
const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(60);

/// Default cap on retained response bytes (the spec's `capture_cap`).
const DEFAULT_RAW_CAP: usize = 50 * 1024 * 1024;

/// Default number of interim (1xx) responses allowed per exchange.
const DEFAULT_MAX_INTERIM: usize = 16;

/// Sends requests over one [`Connection`] and reads their responses with
/// `crosswire-h1`, one exchange at a time.
///
/// The session owns the response parser and the bytes read but not yet
/// consumed, so a keep-alive connection can carry several exchanges.
#[derive(Debug)]
pub struct Session {
    pub(crate) conn: Connection,
    pub(crate) parser_cfg: ParserConfig,
    pub(crate) parser: Parser,
    /// Bytes read and not yet consumed by the parser: the carry that is
    /// re-presented with the next read.
    pub(crate) pending: BytesMut,
    exchanges: u64,
    info: ConnectionInfo,
    pub(crate) read_timeout: Duration,
    pub(crate) raw_cap: usize,
    pub(crate) max_interim: usize,
    pub(crate) reusable: bool,
    pub(crate) in_flight: bool,
}

impl Session {
    /// Wraps a connection. Responses are parsed with `cfg`.
    #[must_use]
    pub fn new(conn: Connection, cfg: ParserConfig) -> Session {
        let info = conn.info().clone();
        let parser = Parser::response(&cfg, ResponseContext::for_method(b"GET"));
        Session {
            conn,
            parser_cfg: cfg,
            parser,
            pending: BytesMut::new(),
            exchanges: 0,
            info,
            read_timeout: DEFAULT_READ_TIMEOUT,
            raw_cap: DEFAULT_RAW_CAP,
            max_interim: DEFAULT_MAX_INTERIM,
            reusable: true,
            in_flight: false,
        }
    }

    /// Sets how long one read or write may stall (default 60 s).
    #[must_use]
    pub fn with_read_timeout(mut self, timeout: Duration) -> Session {
        self.read_timeout = timeout;
        self
    }

    /// Caps how many response bytes each stream retains for
    /// [`raw_response`](ResponseStream::raw_response) (default 50 MiB). Body
    /// pieces keep streaming past the cap; only the retained copy stops.
    #[must_use]
    pub fn with_raw_cap(mut self, bytes: usize) -> Session {
        self.raw_cap = bytes;
        self
    }

    /// Sets how many interim (1xx) responses one exchange may receive
    /// (default 16). One more fails with [`ClientError::Protocol`] and the
    /// session stops being reusable.
    #[must_use]
    pub fn with_max_interim(mut self, n: usize) -> Session {
        self.max_interim = n;
        self
    }

    /// The connection facts for the latest exchange; `reused` is true from
    /// the second exchange on.
    #[must_use]
    pub fn info(&self) -> &ConnectionInfo {
        &self.info
    }

    /// Whether another request may be sent on this connection. False after
    /// `Connection: close` (either side), HTTP/1.0 without keep-alive,
    /// until-close or upgraded framing, bytes beyond the response, any
    /// error, or while a response is still being read.
    #[must_use]
    pub fn is_reusable(&self) -> bool {
        self.reusable && !self.in_flight
    }

    /// Writes every segment of `plan` in order and returns the stream that
    /// reads the response. `ctx` describes the request for response framing.
    ///
    /// The whole plan is written before any response byte is read, so an
    /// `Expect: 100-continue` body is still sent up front in this version.
    ///
    /// # Errors
    /// [`ClientError::Protocol`] when the plan has a hold, the previous
    /// response was not read to its end, bytes were left after it, or the
    /// connection is no longer reusable; [`ClientError::Timeout`] or
    /// [`ClientError::Io`] when writing fails.
    pub async fn send(
        &mut self,
        plan: WritePlan,
        ctx: ResponseContext,
    ) -> Result<ResponseStream<'_>, ClientError> {
        if plan.segments().iter().any(|s| s.hold.is_some()) {
            return Err(ClientError::Protocol("holds are not supported yet".into()));
        }
        if self.in_flight {
            return Err(ClientError::Protocol(
                "previous response not read to its end".into(),
            ));
        }
        if !self.pending.is_empty() {
            return Err(ClientError::Protocol(
                "unexpected bytes after response".into(),
            ));
        }
        if self.exchanges > 0 && !self.reusable {
            return Err(ClientError::Protocol("connection is not reusable".into()));
        }
        self.reusable = false;
        self.in_flight = true;
        let mut request = Vec::new();
        for seg in plan.segments() {
            self.write_all(&seg.bytes).await?;
            request.extend_from_slice(&seg.bytes);
        }
        tokio::time::timeout(self.read_timeout, self.conn.io_mut().flush())
            .await
            .map_err(|_| ClientError::Timeout)??;
        let written = Instant::now();
        self.info = self
            .conn
            .info()
            .clone()
            .with_conn(self.conn.info().conn_id(), self.exchanges > 0);
        self.exchanges += 1;
        self.parser.reset_response(ctx);
        let request_closes = request_closes_connection(&request, &self.parser_cfg);
        Ok(ResponseStream::new(
            self,
            request,
            request_closes,
            written,
            ctx,
        ))
    }

    async fn write_all(&mut self, bytes: &[u8]) -> Result<(), ClientError> {
        tokio::time::timeout(self.read_timeout, self.conn.io_mut().write_all(bytes))
            .await
            .map_err(|_| ClientError::Timeout)??;
        Ok(())
    }
}

/// Whether the request asks for the connection to be closed after the
/// response. A request whose head cannot be read counts as closing: nothing
/// is known about what the server will do with it.
fn request_closes_connection(request: &[u8], cfg: &ParserConfig) -> bool {
    let mut p = Parser::request(cfg);
    match p.feed(request) {
        Ok(step) => match step.event {
            Progress::Head(msg) => close_semantics(&msg).closes(),
            _ => true,
        },
        Err(_) => true,
    }
}
