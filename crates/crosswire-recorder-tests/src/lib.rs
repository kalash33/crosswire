//! Integration tests spanning the Crosswire recorder crates, and the
//! reference client-side recording recipe.
//!
//! [`record_exchange`] is how a client-side tool (the repeater) records what
//! it sends: send a request over a [`Session`], read the response, and hand
//! an [`Exchange`] built from the exact bytes written and read to an
//! [`ExchangeSink`]. The proxy does not use it: its relay has its own
//! capture, capture cap and in-flight budget (`crosswire-proxy`, ADR-0003).

use std::sync::Arc;
use std::time::Instant;

use bytes::Bytes;
use crosswire_client::{ClientError, Session, WritePlan};
use crosswire_h1::{ParseError, Parser, ParserConfig, Progress, ResponseContext};
use crosswire_model::{Body, Exchange, ExchangeSink, Message, SinkError, Timing};
use ulid::Ulid;

/// Why [`record_exchange`] failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RecordError {
    /// Sending the request or reading the response failed.
    #[error("client: {0}")]
    Client(#[from] ClientError),
    /// The bytes written do not parse as a request head.
    #[error("request head: {0}")]
    RequestHead(String),
    /// The response outgrew the session's raw cap (at this many bytes), so
    /// the retained copy is incomplete and is not recorded as if it were whole.
    #[error("response exceeded the raw cap at {0} bytes")]
    Truncated(u64),
    /// The blocking task that submits to the sink did not finish.
    #[error("submit task: {0}")]
    Join(String),
    /// The sink refused the exchange.
    #[error("sink: {0}")]
    Sink(#[from] SinkError),
}

impl From<ParseError> for RecordError {
    fn from(e: ParseError) -> Self {
        RecordError::RequestHead(e.to_string())
    }
}

/// Sends `plan` over `session`, reads the whole response and submits the
/// exchange to `sink`; returns the exchange id.
///
/// What is recorded is what crossed the wire, never a re-serialisation:
///
/// - the request is the bytes the session wrote
///   ([`request_bytes`](crosswire_client::ResponseStream::request_bytes)),
///   split at the end of the parsed head; the rest is the raw request body;
/// - the response head is the parsed head, and its body is
///   [`raw_response`](crosswire_client::ResponseStream::raw_response) after
///   the head (chunk framing, last chunk and trailers included), so the
///   stored body is byte-for-byte what the server sent;
/// - both messages keep `fidelity = Raw` and carry [`Session::info`];
/// - timings come from the connection (`connect_ms`, `tls_ms`) and the
///   stream (`ttfb_ms`, and total time from before the send to the last
///   body byte).
///
/// The body is attached as [`Body::Inline`]; the store decides whether it
/// stays inline or goes to a pack.
///
/// `ExchangeSink::submit` may block on file I/O, so it runs on
/// `tokio::task::spawn_blocking` with a clone of `sink`. That works on
/// current-thread runtimes, multi-thread runtimes and inside a
/// `tokio::task::LocalSet`, and never blocks the executor. A successful
/// return means the exchange was accepted, not that it is durable: wait for
/// the store's `Event::Committed` carrying the returned id.
///
/// # Errors
/// [`RecordError::Client`] when the exchange fails on the wire,
/// [`RecordError::RequestHead`] when the written bytes are not a request
/// head, [`RecordError::Truncated`] when the response exceeded the
/// session's raw cap, [`RecordError::Sink`] when the sink refuses the exchange,
/// [`RecordError::Join`] when the submit task panics or is cancelled.
pub async fn record_exchange(
    session: &mut Session,
    plan: WritePlan,
    ctx: ResponseContext,
    sink: Arc<dyn ExchangeSink>,
) -> Result<Ulid, RecordError> {
    // A failure partway through the response records nothing: the `?`s below
    // drop the half-read exchange. Plan 3's relay should record an error
    // exchange (request plus the failure) instead.
    let started = Instant::now();
    let (connect_ms, tls_ms) = {
        let c = session.connection();
        (c.connect_ms(), c.tls_ms())
    };
    let mut stream = session.send(plan, ctx).await?;

    let written = Bytes::copy_from_slice(stream.request_bytes());
    let mut parser = Parser::request(&ParserConfig::default());
    let step = parser.feed(&written)?;
    let Progress::Head(req_head) = step.event else {
        return Err(RecordError::RequestHead("incomplete head".into()));
    };
    let req_body = written.slice(step.consumed..);

    let resp_head = stream.head().await?;
    while stream.next_body().await?.is_some() {}
    // The raw response is interim heads, the final head, then the framed
    // body. Summing the head lengths finds where the body starts; the last
    // chunk and trailers arrive after the final payload piece, so only the
    // retained raw bytes (not the pieces) hold the complete framing.
    if let Some(at) = stream.raw_truncated_at() {
        return Err(RecordError::Truncated(at));
    }
    let head_len: usize = stream
        .interim_responses()
        .iter()
        .map(|m| m.bytes().len())
        .sum::<usize>()
        + resp_head.bytes().len();
    let resp_body = Bytes::copy_from_slice(stream.raw_response().get(head_len..).unwrap_or(&[]));
    let timing = Timing {
        connect_ms,
        tls_ms,
        ttfb_ms: stream.ttfb_ms(),
        total_ms: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
    };
    drop(stream);
    // After the send: the facts for this exchange (`reused` is set by then).
    let info = session.info().clone();

    let request = Message::http1_raw(&req_head, inline(req_body), info.clone());
    let response = Message::http1_raw(&resp_head, inline(resp_body), info);
    let mut exchange = Exchange::new(request);
    exchange.set_response(response);
    exchange.set_timing(timing);
    let id = exchange.id();
    let accepted = tokio::task::spawn_blocking(move || sink.submit(exchange)).await;
    accepted.map_err(|e| RecordError::Join(e.to_string()))??;
    Ok(id)
}

fn inline(bytes: Bytes) -> Body {
    if bytes.is_empty() {
        Body::None
    } else {
        Body::inline(bytes)
    }
}
