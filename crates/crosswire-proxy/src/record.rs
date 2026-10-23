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

/// Least time between two "sink refused" warnings.
const REFUSAL_WARN_EVERY: Duration = Duration::from_secs(10);

/// Turns relay drafts into exchanges and submits them, and markers, to the
/// sink from blocking threads. One per listener; cheap to clone.
#[derive(Clone)]
pub(crate) struct Recorder {
    sink: Arc<dyn ExchangeSink>,
    counters: Arc<Counters>,
    parse: ParserConfig,
    /// Throttles the "sink refused" warning.
    warn_gate: Arc<WarnGate>,
}

/// Lets a warning through at most once per [`REFUSAL_WARN_EVERY`].
#[derive(Debug, Default)]
pub(crate) struct WarnGate {
    last: Mutex<Option<Instant>>,
}

impl WarnGate {
    /// Whether a warning at `now` should be logged; if so, `now` starts
    /// the next quiet period.
    pub(crate) fn should_log(&self, now: Instant) -> bool {
        let mut last = self.last.lock().unwrap_or_else(PoisonError::into_inner);
        let open = last.is_none_or(|t| now.saturating_duration_since(t) >= REFUSAL_WARN_EVERY);
        if open {
            *last = Some(now);
        }
        open
    }
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
            warn_gate: Arc::new(WarnGate::default()),
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

    /// Counts a full sink in `sink_full`; logs any refusal at most once
    /// per [`REFUSAL_WARN_EVERY`]. Never logs the record's bytes.
    fn refused(&self, e: SinkError, what: &'static str) {
        if e == SinkError::Full {
            self.counters.sink_full.fetch_add(1, Ordering::Relaxed);
        }
        if self.warn_gate.should_log(Instant::now()) {
            let sink_full = self.counters.sink_full.load(Ordering::Relaxed);
            tracing::warn!(error = %e, what, sink_full, "sink refused a record; further refusals are not logged for 10 s");
        }
    }
}

fn reparse(cfg: &ParserConfig, head: &[u8]) -> Option<RawMessage> {
    match Parser::request(cfg).feed(head).ok()?.event {
        Progress::Head(m) => Some(m),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warn_gate_logs_once_per_ten_seconds() {
        let gate = WarnGate::default();
        let t0 = Instant::now();
        assert!(gate.should_log(t0));
        assert!(!gate.should_log(t0 + Duration::from_secs(1)));
        assert!(!gate.should_log(t0 + Duration::from_millis(9_999)));
        assert!(gate.should_log(t0 + Duration::from_secs(10)));
        assert!(!gate.should_log(t0 + Duration::from_secs(15)));
        assert!(gate.should_log(t0 + Duration::from_secs(20)));
    }
}
