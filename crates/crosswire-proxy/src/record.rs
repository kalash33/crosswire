//! Building exchanges from what the relay carried and handing them, and
//! markers, to the sink without blocking the relay.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use crosswire_h1::{Parser, ParserConfig, Progress, RawMessage};
use crosswire_model::{
    ConnectionInfo, Exchange, ExchangeSink, Marker, MarkerKind, Message, SinkError,
};

use crate::capture::Reservation;
use crate::relay::ExchangeDraft;
use crate::Counters;

/// Least time between two "sink is full" warnings.
const FULL_WARN_EVERY: Duration = Duration::from_secs(10);

/// Turns relay drafts into exchanges and submits them, and markers, to the
/// sink from blocking threads. One per listener; cheap to clone.
#[derive(Clone)]
pub(crate) struct Recorder {
    sink: Arc<dyn ExchangeSink>,
    counters: Arc<Counters>,
    parse: ParserConfig,
    /// When "sink is full" was last logged.
    full_warned: Arc<Mutex<Option<Instant>>>,
}

/// What a tunnel needs to record what it carried.
#[derive(Clone)]
pub(crate) struct RecordCtx {
    /// Where exchanges and markers go.
    pub(crate) recorder: Recorder,
    /// The client-side connection record for this tunnel.
    pub(crate) info: ConnectionInfo,
}

impl RecordCtx {
    /// Submits `exchange` without waiting (see [`Recorder::submit`]).
    pub(crate) fn submit(&self, exchange: Exchange) {
        self.recorder.submit(exchange);
    }
}

impl Recorder {
    /// A recorder submitting to `sink`, counting refusals in `counters`,
    /// re-parsing request heads with `parse` (the proxy's parser config).
    pub(crate) fn new(
        sink: Arc<dyn ExchangeSink>,
        counters: Arc<Counters>,
        parse: ParserConfig,
    ) -> Recorder {
        Recorder {
            sink,
            counters,
            parse,
            full_warned: Arc::new(Mutex::new(None)),
        }
    }

    /// Builds the exchange for a relayed `draft` and submits it without
    /// waiting. The request head is re-parsed from the rewritten bytes
    /// (what went upstream) with the proxy's parser config, so its spans
    /// match the recorded bytes; the diffs index the head as the client
    /// sent it. The bodies' budget reservations are released once the
    /// sink has taken (or refused) the exchange.
    pub(crate) fn record(&self, draft: ExchangeDraft) {
        let ExchangeDraft {
            req_head,
            req_rewritten,
            diffs,
            mut req_body,
            resp_head,
            interim,
            mut resp_body,
            timing,
            info,
            state,
            generated,
            body_diagnostics: (req_body_diags, resp_body_diags),
        } = draft;
        let req_head = match reparse(&self.parse, &req_rewritten) {
            Some(head) => head,
            None => {
                // Recorded as the client sent it; the marker makes the
                // mismatch with what went upstream visible.
                self.submit_marker(Marker::new(
                    MarkerKind::Error,
                    "rewritten request head did not re-parse",
                ));
                req_head
            }
        };
        let mut req_diags = req_head.diagnostics().to_vec();
        req_diags.extend(req_body_diags);
        let request = Message::http1_raw(&req_head, req_body.body(), info.clone())
            .with_diffs(diffs)
            .with_diagnostics(req_diags);
        let mut exchange = Exchange::new(request);
        let response = match (generated, resp_head) {
            (Some(generated), _) => Some(generated),
            (None, Some(head)) => {
                let mut diags = head.diagnostics().to_vec();
                diags.extend(resp_body_diags);
                Some(
                    Message::http1_raw(&head, resp_body.body(), info)
                        .with_diagnostics(diags)
                        .with_interim(interim),
                )
            }
            (None, None) => None,
        };
        if let Some(response) = response {
            exchange.set_response(response);
        }
        exchange.set_timing(timing);
        exchange.set_state(state);
        let held = [req_body.reservation.take(), resp_body.reservation.take()];
        self.submit_holding(exchange, held);
    }

    /// Submits `exchange` from a blocking thread. Never awaited: the relay
    /// does not wait on the store (spec 9.3). A refusal is logged and, for
    /// a full sink, counted.
    pub(crate) fn submit(&self, exchange: Exchange) {
        self.submit_holding(exchange, [None, None]);
    }

    fn submit_holding(&self, exchange: Exchange, held: [Option<Reservation>; 2]) {
        let this = self.clone();
        drop(tokio::task::spawn_blocking(move || {
            let result = this.sink.submit(exchange);
            drop(held);
            if let Err(e) = result {
                this.refused(e, "exchange");
            }
        }));
    }

    /// Submits `marker` from a blocking thread without waiting.
    pub(crate) fn submit_marker(&self, marker: Marker) {
        let this = self.clone();
        drop(tokio::task::spawn_blocking(move || {
            if let Err(e) = this.sink.submit_marker(marker) {
                this.refused(e, "marker");
            }
        }));
    }

    /// Counts a full sink (logged at most once per [`FULL_WARN_EVERY`]);
    /// logs any other refusal. Never logs the record's bytes.
    fn refused(&self, e: SinkError, what: &'static str) {
        if e != SinkError::Full {
            tracing::warn!(error = %e, what, "sink refused a record");
            return;
        }
        let n = self.counters.sink_full.fetch_add(1, Ordering::Relaxed) + 1;
        let now = Instant::now();
        let mut last = self
            .full_warned
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if last.is_none_or(|t| now.duration_since(t) >= FULL_WARN_EVERY) {
            *last = Some(now);
            drop(last);
            tracing::warn!(
                what,
                sink_full = n,
                "sink is full; records are being dropped"
            );
        }
    }
}

fn reparse(cfg: &ParserConfig, head: &[u8]) -> Option<RawMessage> {
    match Parser::request(cfg).feed(head).ok()?.event {
        Progress::Head(m) => Some(m),
        _ => None,
    }
}
