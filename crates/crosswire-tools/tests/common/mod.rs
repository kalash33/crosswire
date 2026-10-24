//! Shared test harness for the repeater integration tests.
//!
//! `upstream` holds in-process raw and TLS servers copied from the proxy's
//! `tests/common/upstream.rs`. A [`TestSink`] captures every submitted
//! exchange, and [`AuditLog`] records the repeater's audit calls.
#![allow(dead_code, missing_docs)]

pub mod upstream;

use std::sync::{Arc, Mutex};

use crosswire_model::{Exchange, ExchangeSink, Marker, SinkError};

/// An [`ExchangeSink`] that keeps every accepted exchange in memory.
#[derive(Default)]
pub struct TestSink {
    exchanges: Mutex<Vec<Exchange>>,
}

impl TestSink {
    /// A fresh, empty sink.
    pub fn new() -> Arc<TestSink> {
        Arc::new(TestSink::default())
    }

    /// A snapshot of every exchange accepted so far.
    pub fn exchanges(&self) -> Vec<Exchange> {
        self.exchanges.lock().unwrap().clone()
    }

    /// How many exchanges have been accepted.
    pub fn len(&self) -> usize {
        self.exchanges.lock().unwrap().len()
    }
}

impl ExchangeSink for TestSink {
    fn submit(&self, exchange: Exchange) -> Result<(), SinkError> {
        self.exchanges.lock().unwrap().push(exchange);
        Ok(())
    }

    fn submit_marker(&self, _marker: Marker) -> Result<(), SinkError> {
        Ok(())
    }
}

/// The repeater's audit-hook type.
pub type AuditHook = Arc<dyn Fn(&str, &str, &str) + Send + Sync>;

/// Records the `(action, target, result)` tuples passed to the repeater's
/// audit hook.
#[derive(Clone, Default)]
pub struct AuditLog {
    calls: Arc<Mutex<Vec<(String, String, String)>>>,
}

impl AuditLog {
    /// A fresh, empty log.
    pub fn new() -> AuditLog {
        AuditLog::default()
    }

    /// An audit closure that appends to this log.
    pub fn hook(&self) -> AuditHook {
        let calls = Arc::clone(&self.calls);
        Arc::new(move |action: &str, target: &str, result: &str| {
            calls.lock().unwrap().push((
                action.to_string(),
                target.to_string(),
                result.to_string(),
            ));
        })
    }

    /// A snapshot of every recorded call.
    pub fn calls(&self) -> Vec<(String, String, String)> {
        self.calls.lock().unwrap().clone()
    }

    /// Whether a call with this action was recorded.
    pub fn has_action(&self, action: &str) -> bool {
        self.calls.lock().unwrap().iter().any(|c| c.0 == action)
    }
}
