//! Building exchanges from what the relay carried and handing them, and
//! markers, to the sink without blocking the relay.
//!
//! Records go through a bounded queue to one dedicated recorder thread,
//! which calls the sink (and may block on its journal I/O). The relay only
//! ever does a `try_send`: when the queue is full the record is dropped,
//! counted in `sink_full`, and an exchange's loss is marked with a `Gap`
//! marker for its id. The thread never touches tokio's blocking pool, so a
//! slow store cannot hold up DNS lookups and new connections (spec 9.3).

use std::sync::atomic::Ordering;
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
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
/// Records waiting for the recorder thread.
pub(crate) const RECORD_QUEUE: usize = 1024;

/// One record for the recorder thread.
enum Job {
    /// An exchange and the budget reservations of its bodies, released
    /// once the sink has taken (or refused) it.
    Exchange(Box<Exchange>, [Option<Reservation>; 2]),
    Marker(Box<Marker>),
    /// Ends the thread once everything before it is submitted.
    Stop,
}

/// What the recorder thread and the overflow path share.
struct Out {
    sink: Arc<dyn ExchangeSink>,
    counters: Arc<Counters>,
    /// Throttles the "sink refused" warning.
    warn_gate: WarnGate,
}

/// Turns relay drafts into exchanges and queues them, and markers, for the
/// recorder thread. One per listener; cheap to clone.
#[derive(Clone)]
pub(crate) struct Recorder {
    tx: SyncSender<Job>,
    out: Arc<Out>,
    parse: ParserConfig,
}

/// The recorder thread; [`RecorderThread::stop`] drains and joins it.
pub(crate) struct RecorderThread {
    tx: SyncSender<Job>,
    handle: Option<JoinHandle<()>>,
}

impl RecorderThread {
    /// Submits everything queued so far, then ends the thread and waits for
    /// it (on a blocking thread, so the runtime is not held). Records
    /// queued after this are dropped when the last [`Recorder`] goes.
    pub(crate) async fn stop(mut self) {
        let Some(handle) = self.handle.take() else {
            return;
        };
        let tx = self.tx.clone();
        let joined = tokio::task::spawn_blocking(move || {
            // Fails only if the thread is already gone.
            let _ = tx.send(Job::Stop);
            handle.join()
        })
        .await;
        if !matches!(joined, Ok(Ok(()))) {
            tracing::warn!("the recorder thread ended abnormally");
        }
    }
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
    /// A recorder submitting to `sink` from its own thread through a queue
    /// of [`RECORD_QUEUE`] records, counting refusals and overflows in
    /// `counters`, re-parsing request heads with `parse` (the proxy's parser
    /// config).
    pub(crate) fn start(
        sink: Arc<dyn ExchangeSink>,
        counters: Arc<Counters>,
        parse: ParserConfig,
    ) -> std::io::Result<(Recorder, RecorderThread)> {
        Recorder::with_capacity(sink, counters, parse, RECORD_QUEUE)
    }

    fn with_capacity(
        sink: Arc<dyn ExchangeSink>,
        counters: Arc<Counters>,
        parse: ParserConfig,
        capacity: usize,
    ) -> std::io::Result<(Recorder, RecorderThread)> {
        let (tx, rx) = sync_channel(capacity);
        let out = Arc::new(Out {
            sink,
            counters,
            warn_gate: WarnGate::default(),
        });
        let thread_out = Arc::clone(&out);
        let handle = std::thread::Builder::new()
            .name("crosswire-recorder".into())
            .spawn(move || thread_out.run(&rx))?;
        let recorder = Recorder {
            tx: tx.clone(),
            out,
            parse,
        };
        Ok((
            recorder,
            RecorderThread {
                tx,
                handle: Some(handle),
            },
        ))
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
            // Interim heads already reached the client before the
            // generated final response, so they are part of it too.
            (Some(generated), _) => Some(generated.with_interim(interim)),
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

    /// Queues `exchange` for the recorder thread. Never waits: the relay
    /// does not wait on the store (spec 9.3). A full queue drops it (see
    /// [`Recorder::overflow`]); a sink refusal is logged and, for a full
    /// sink, counted.
    pub(crate) fn submit(&self, exchange: Exchange) {
        self.submit_holding(exchange, [None, None]);
    }

    fn submit_holding(&self, exchange: Exchange, held: [Option<Reservation>; 2]) {
        match self.tx.try_send(Job::Exchange(Box::new(exchange), held)) {
            Ok(()) => {}
            Err(TrySendError::Full(Job::Exchange(ex, held))) => {
                drop(held);
                self.overflow(&ex);
            }
            Err(TrySendError::Full(_)) => {}
            Err(TrySendError::Disconnected(_)) => {
                tracing::debug!("recorder stopped; exchange not recorded");
            }
        }
    }

    /// Queues `marker` without waiting; a full queue drops it (counted).
    pub(crate) fn submit_marker(&self, marker: Marker) {
        match self.tx.try_send(Job::Marker(Box::new(marker))) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => self.out.refused(SinkError::Full, "marker"),
            Err(TrySendError::Disconnected(_)) => {
                tracing::debug!("recorder stopped; marker not recorded");
            }
        }
    }

    /// The queue is full: `ex` is dropped. Counted in `sink_full`, warned
    /// about (throttled), and marked with a `Gap` marker for its id,
    /// submitted from a blocking thread (the sink's marker path never
    /// blocks, and this path is rare).
    fn overflow(&self, ex: &Exchange) {
        self.out.refused(SinkError::Full, "exchange");
        let id = ex.id();
        let gap = Marker::new(
            MarkerKind::Gap,
            "recorder queue full: exchange dropped before reaching the store",
        )
        .with_from(id)
        .with_to(id);
        let out = Arc::clone(&self.out);
        let submit = move || {
            if let Err(e) = out.sink.submit_marker(gap) {
                out.refused(e, "gap marker");
            }
        };
        match tokio::runtime::Handle::try_current() {
            Ok(rt) => drop(rt.spawn_blocking(submit)),
            Err(_) => submit(),
        }
    }
}

impl Out {
    /// The recorder thread: submits records in order until told to stop
    /// or every [`Recorder`] is gone.
    fn run(&self, rx: &Receiver<Job>) {
        while let Ok(job) = rx.recv() {
            match job {
                Job::Exchange(ex, held) => {
                    let result = self.sink.submit(*ex);
                    drop(held);
                    if let Err(e) = result {
                        self.refused(e, "exchange");
                    }
                }
                Job::Marker(m) => {
                    if let Err(e) = self.sink.submit_marker(*m) {
                        self.refused(e, "marker");
                    }
                }
                Job::Stop => break,
            }
        }
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
    use crosswire_model::{Body, Fidelity, FramingRecord, Head, MessageKind};
    use std::sync::atomic::AtomicUsize;
    use std::sync::Condvar;
    use ulid::Ulid;

    /// A sink whose `submit` blocks until released; markers never block.
    #[derive(Default)]
    struct GateSink {
        entered: AtomicUsize,
        open: Mutex<bool>,
        opened: Condvar,
        exchanges: Mutex<Vec<Ulid>>,
        markers: Mutex<Vec<Marker>>,
    }

    impl GateSink {
        fn release(&self) {
            *self.open.lock().unwrap() = true;
            self.opened.notify_all();
        }
    }

    impl ExchangeSink for GateSink {
        fn submit(&self, exchange: Exchange) -> Result<(), SinkError> {
            self.entered.fetch_add(1, Ordering::SeqCst);
            let mut open = self.open.lock().unwrap();
            while !*open {
                open = self.opened.wait(open).unwrap();
            }
            self.exchanges.lock().unwrap().push(exchange.id());
            Ok(())
        }

        fn submit_marker(&self, marker: Marker) -> Result<(), SinkError> {
            self.markers.lock().unwrap().push(marker);
            Ok(())
        }
    }

    fn exchange() -> Exchange {
        Exchange::new(Message::new(
            MessageKind::Normalized,
            Fidelity::Normalized,
            Head::Normalized { map: Vec::new() },
            FramingRecord::None,
            Body::None,
            ConnectionInfo::new("u.test", 80),
        ))
    }

    async fn until(what: &str, cond: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !cond() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    }

    #[tokio::test]
    async fn full_queue_drops_with_a_gap_marker_and_never_blocks() {
        let sink = Arc::new(GateSink::default());
        let counters = Arc::new(Counters::default());
        let (rec, thread) = Recorder::with_capacity(
            Arc::clone(&sink) as Arc<dyn ExchangeSink>,
            Arc::clone(&counters),
            ParserConfig::default(),
            2,
        )
        .unwrap();
        let first = exchange();
        let mut kept = vec![first.id()];
        rec.submit(first);
        until("the thread to block in the sink", || {
            sink.entered.load(Ordering::SeqCst) == 1
        })
        .await;
        // Two fit in the queue; the next two overflow. None of this waits.
        let start = Instant::now();
        let mut dropped = Vec::new();
        for i in 0..4 {
            let ex = exchange();
            if i < 2 {
                kept.push(ex.id());
            } else {
                dropped.push(ex.id());
            }
            rec.submit(ex);
        }
        assert!(start.elapsed() < Duration::from_secs(1), "submit blocked");
        assert_eq!(counters.sink_full.load(Ordering::SeqCst), 2);
        until("a gap marker per dropped exchange", || {
            sink.markers.lock().unwrap().len() == 2
        })
        .await;
        // One gap marker per dropped id, in either order (each is
        // submitted from its own blocking task).
        let mut marked: Vec<Ulid> = Vec::new();
        for m in sink.markers.lock().unwrap().iter() {
            assert_eq!(m.kind(), MarkerKind::Gap);
            assert_eq!(m.from_ulid(), m.to_ulid());
            marked.push(m.from_ulid().expect("a gap marker names its exchange"));
        }
        marked.sort();
        dropped.sort();
        assert_eq!(marked, dropped);
        sink.release();
        thread.stop().await;
        assert_eq!(*sink.exchanges.lock().unwrap(), kept, "kept ones, in order");
        // After stop, records are dropped quietly.
        rec.submit(exchange());
        assert_eq!(sink.exchanges.lock().unwrap().len(), 3);
    }

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
