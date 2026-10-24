//! The client-side recording recipe.
//!
//! [`record_exchange`] is how a client-side tool (the repeater) records what
//! it sends: it sends a request over a [`Session`], reads the whole response,
//! and hands an [`Exchange`] built from the exact bytes written and read to an
//! [`ExchangeSink`]. It was first written as the reference recipe in
//! `crosswire-recorder-tests`; it moved here, and that crate now re-exports a
//! thin four-argument wrapper so its tests keep compiling. The proxy does not
//! use it: its relay has its own capture, capture cap and in-flight budget
//! (`crosswire-proxy`, ADR-0003).

use std::sync::Arc;
use std::time::Instant;

use bytes::Bytes;
use crosswire_client::{ClientError, Session, WritePlan};
use crosswire_h1::{ParseError, Parser, ParserConfig, Progress, ResponseContext};
use crosswire_model::{
    Body, Diff, Exchange, ExchangeSink, Message, Namespace, ScopeState, SinkError, Timing,
};
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

/// What [`record_exchange`] returns on success: the recorded exchange id, its
/// timing, whether a failing certificate chain was accepted, and the response
/// status and `Location` header a redirect follower needs.
#[derive(Clone, Debug)]
pub struct Recorded {
    /// The id of the submitted exchange.
    pub id: Ulid,
    /// The measured timing of the exchange.
    pub timing: Timing,
    /// Whether the connection accepted an invalid certificate chain.
    pub cert_override: bool,
    /// The response status code, if one was parsed.
    pub status: Option<u16>,
    /// The `Location` header value, if present.
    pub location: Option<String>,
}

/// Sends `plan` over `session`, reads the whole response and submits the
/// exchange to `sink`; returns the [`Recorded`] facts.
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
/// The exchange is filed in `namespace` with `scope_state`; `diffs` are
/// attached to the request message, and `source` (a prior exchange this send
/// was derived from) is recorded as a `source:<ulid>` tag, since the schema
/// has no dedicated column for it yet.
///
/// The body is attached as [`Body::Inline`]; the store decides whether it
/// stays inline or goes to a pack.
///
/// `ExchangeSink::submit` may block on file I/O, so it runs on
/// `tokio::task::spawn_blocking` with a clone of `sink`. A successful return
/// means the exchange was accepted, not that it is durable: wait for the
/// store's `Event::Committed` carrying the returned id.
///
/// # Errors
/// [`RecordError::Client`] when the exchange fails on the wire,
/// [`RecordError::RequestHead`] when the written bytes are not a request
/// head, [`RecordError::Truncated`] when the response exceeded the
/// session's raw cap, [`RecordError::Sink`] when the sink refuses the
/// exchange, [`RecordError::Join`] when the submit task panics or is cancelled.
#[allow(clippy::too_many_arguments)]
pub async fn record_exchange(
    session: &mut Session,
    plan: WritePlan,
    ctx: ResponseContext,
    sink: Arc<dyn ExchangeSink>,
    namespace: Namespace,
    scope_state: ScopeState,
    diffs: Vec<Diff>,
    source: Option<Ulid>,
) -> Result<Recorded, RecordError> {
    // A failure partway through the response records nothing: the `?`s below
    // drop the half-read exchange.
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
    let cert_override = info.cert_override();

    // The status and Location a redirect follower needs, read before the head
    // is moved into the request-response pair.
    let status = resp_head.head().status_code(resp_head.bytes());
    let location = resp_head
        .head()
        .header(resp_head.bytes(), "location")
        .and_then(|span| resp_head.bytes().get(span.range()))
        .map(|v| String::from_utf8_lossy(v).trim().to_string());

    let request = Message::http1_raw(&req_head, inline(req_body), info.clone()).with_diffs(diffs);
    let response = Message::http1_raw(&resp_head, inline(resp_body), info);
    let mut exchange = Exchange::new(request);
    exchange.set_response(response);
    exchange.set_timing(timing);
    exchange.set_namespace(namespace);
    exchange.set_scope_state(scope_state);
    if let Some(src) = source {
        exchange.add_tag(format!("source:{src}"));
    }
    let id = exchange.id();
    let accepted = tokio::task::spawn_blocking(move || sink.submit(exchange)).await;
    accepted.map_err(|e| RecordError::Join(e.to_string()))??;
    Ok(Recorded {
        id,
        timing,
        cert_override,
        status,
        location,
    })
}

fn inline(bytes: Bytes) -> Body {
    if bytes.is_empty() {
        Body::None
    } else {
        Body::inline(bytes)
    }
}
