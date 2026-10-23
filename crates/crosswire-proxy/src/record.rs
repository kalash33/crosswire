//! Handing finished exchanges and markers to the sink without blocking the
//! relay.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use crosswire_h1::{Parser, ParserConfig, Progress, RawMessage};
use crosswire_model::{
    Body, ConnectionInfo, Exchange, ExchangeSink, Marker, MarkerKind, Message, SinkError,
};

use crate::capture::CaptureResult;
use crate::relay::ExchangeDraft;
use crate::Counters;

/// What a tunnel or relay needs to record what it carried.
#[derive(Clone)]
pub(crate) struct RecordCtx {
    /// Where exchanges and markers go.
    pub(crate) sink: Arc<dyn ExchangeSink>,
    /// `sink_full` is counted here.
    pub(crate) counters: Arc<Counters>,
    /// The client-side connection record for this tunnel or relay.
    pub(crate) info: ConnectionInfo,
}

impl RecordCtx {
    /// Submits `exchange` from a blocking thread. Never awaited: the
    /// relay does not wait on the store (spec 9.3). A refusal is logged
    /// and, for a full sink, counted.
    pub(crate) fn submit(&self, exchange: Exchange) {
        submit_exchange(&self.sink, &self.counters, exchange);
    }
}

/// Submits `exchange` to `sink` on a blocking thread without waiting.
pub(crate) fn submit_exchange(
    sink: &Arc<dyn ExchangeSink>,
    counters: &Arc<Counters>,
    exchange: Exchange,
) {
    let (sink, counters) = (Arc::clone(sink), Arc::clone(counters));
    drop(tokio::task::spawn_blocking(move || {
        if let Err(e) = sink.submit(exchange) {
            refused(&counters, e, "exchange");
        }
    }));
}

/// Submits `marker` to `sink` on a blocking thread without waiting.
pub(crate) fn submit_marker(
    sink: &Arc<dyn ExchangeSink>,
    counters: &Arc<Counters>,
    marker: Marker,
) {
    let (sink, counters) = (Arc::clone(sink), Arc::clone(counters));
    drop(tokio::task::spawn_blocking(move || {
        if let Err(e) = sink.submit_marker(marker) {
            refused(&counters, e, "marker");
        }
    }));
}

fn refused(counters: &Counters, e: SinkError, what: &'static str) {
    if e == SinkError::Full {
        counters.sink_full.fetch_add(1, Ordering::Relaxed);
    }
    tracing::warn!(error = %e, what, "sink refused a record");
}

/// Builds the exchange for a relayed `draft` and submits it without
/// waiting. The request head is re-parsed from the rewritten bytes (what
/// went upstream) so its spans match the recorded bytes; the diffs index
/// the head as the client sent it. Task 6 replaces this with the Recorder.
pub(crate) fn record_draft(
    sink: &Arc<dyn ExchangeSink>,
    counters: &Arc<Counters>,
    parse: &ParserConfig,
    draft: ExchangeDraft,
) {
    let req_head = match reparse(parse, &draft.req_rewritten) {
        Some(head) => head,
        None => {
            // Recorded as the client sent it; the marker makes the
            // mismatch with what went upstream visible.
            submit_marker(
                sink,
                counters,
                Marker::new(MarkerKind::Error, "rewritten request head did not re-parse"),
            );
            draft.req_head
        }
    };
    let (req_body_diags, resp_body_diags) = draft.body_diagnostics;
    let mut req_diags = req_head.diagnostics().to_vec();
    req_diags.extend(req_body_diags);
    let request = Message::http1_raw(&req_head, body(&draft.req_body), draft.info.clone())
        .with_diffs(draft.diffs)
        .with_diagnostics(req_diags);
    let mut exchange = Exchange::new(request);
    let response = match (draft.generated, draft.resp_head) {
        (Some(generated), _) => Some(generated),
        (None, Some(head)) => {
            let mut diags = head.diagnostics().to_vec();
            diags.extend(resp_body_diags);
            Some(
                Message::http1_raw(&head, body(&draft.resp_body), draft.info)
                    .with_diagnostics(diags)
                    .with_interim(draft.interim),
            )
        }
        (None, None) => None,
    };
    if let Some(response) = response {
        exchange.set_response(response);
    }
    exchange.set_timing(draft.timing);
    exchange.set_state(draft.state);
    submit_exchange(sink, counters, exchange);
}

fn body(c: &CaptureResult) -> Body {
    if c.len == 0 {
        Body::None
    } else {
        Body::inline(c.bytes.clone())
    }
}

fn reparse(cfg: &ParserConfig, head: &[u8]) -> Option<RawMessage> {
    match Parser::request(cfg).feed(head).ok()?.event {
        Progress::Head(m) => Some(m),
        _ => None,
    }
}
