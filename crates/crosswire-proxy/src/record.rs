//! Handing finished exchanges and markers to the sink without blocking the
//! relay.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use crosswire_model::{ConnectionInfo, Exchange, ExchangeSink, Marker, SinkError};

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
