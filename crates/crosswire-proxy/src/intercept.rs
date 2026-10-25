//! The request-intercept service: holds requests for an operator to
//! forward (unchanged or edited), drop, or release in bulk.
//!
//! [`Intercept`] implements [`Handler`]: the relay calls
//! [`Intercept::on_request`] for every request, and the API (Task 6) calls
//! [`forward`](Intercept::forward), [`drop`](Intercept::drop) and
//! [`forward_all`](Intercept::forward_all) to resolve the held futures. The
//! service is constructed by the server and shared, behind an `Arc`, with
//! both the proxy (as the handler) and the API.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::{Duration, Instant};

use bytes::Bytes;
use crosswire_core::{Change, ChangeOp, ChangeReason, Span};
use crosswire_h1::{EditError, ParseError, Parser, ParserConfig, Progress, RawMessage, StartLine};
use crosswire_model::{ConnectionInfo, ExchangeSink, ScopeState};
use crosswire_scope::Compiled;
use tokio::sync::{oneshot, watch};
use ulid::Ulid;

use crate::capture::Budget;
use crate::handler::{BoxFuture, Decision, Handler, HeldRequest};
use crate::ProxyLimits;

/// What to do with the oldest/newest held request when the queue is full.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Overflow {
    /// Forward the incoming (newest) request without holding it.
    ForwardNewest,
    /// Forward the oldest held request to make room for the newest.
    ForwardOldest,
}

/// What to do when a held request's hold timeout elapses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum OnTimeout {
    /// Forward the request unchanged.
    Forward,
    /// Answer the client with a proxy-generated 504.
    Respond504,
}

/// Which requests to skip (forward without holding) and how to behave when
/// the hold queue is full or a hold times out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InterceptRules {
    /// Path extensions (without the dot) never held, e.g. `png`, `css`.
    pub skip_extensions: Vec<String>,
    /// Skip requests the scope marks out of scope.
    pub skip_out_of_scope: bool,
    /// What to do when the hold queue is full.
    pub overflow: Overflow,
    /// What to do when a hold times out.
    pub on_timeout: OnTimeout,
}

impl Default for InterceptRules {
    fn default() -> InterceptRules {
        InterceptRules {
            skip_extensions: [
                "png", "jpg", "jpeg", "gif", "webp", "svg", "ico", "woff", "woff2", "ttf", "css",
                "js", "map",
            ]
            .iter()
            .map(|s| (*s).to_string())
            .collect(),
            // Spec §4.6: out-of-scope traffic is never intercepted. A no-op
            // under an empty scope, where every exchange is `Empty`, not `Out`.
            skip_out_of_scope: true,
            overflow: Overflow::ForwardNewest,
            on_timeout: OnTimeout::Forward,
        }
    }
}

/// Edited request bytes supplied when forwarding a held request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditedBytes {
    /// The replacement request head.
    pub head: Bytes,
    /// The replacement request body, if any.
    pub body: Option<Bytes>,
}

/// The original buffered bytes of a held request, the baseline the UI edits
/// for byte-exact edit-and-forward and send-to-repeater-from-held.
#[derive(Clone, Debug)]
pub struct HeldBytes {
    /// The original request head bytes (always buffered).
    pub head: Bytes,
    /// The original request body bytes, or `None` when the body was not
    /// buffered because it exceeded the hold limit (an `editable: false`
    /// held request).
    pub body: Option<Bytes>,
}

/// A summary of one held request, for the operator's queue view.
#[derive(Clone, Debug)]
pub struct HeldSummary {
    /// The hold id.
    pub id: Ulid,
    /// The `Host` (or connection target) the request addresses.
    pub host: String,
    /// The request method.
    pub method: String,
    /// The request target (path).
    pub path: String,
    /// The buffered body size in bytes (0 when none was buffered).
    pub size: usize,
    /// Whether the request may be edited (its body was buffered).
    pub editable: bool,
    /// When the request was first held.
    pub held_since: Instant,
}

/// Why a [`forward`](Intercept::forward) or [`drop`](Intercept::drop) call
/// failed.
#[derive(Debug)]
#[non_exhaustive]
pub enum InterceptError {
    /// No held request has that id (already forwarded, dropped, or timed
    /// out).
    NotHeld,
    /// The request's body was not buffered, so it cannot be edited.
    NotEditable,
    /// The edited bytes do not parse as a request.
    Edit(EditError),
}

impl std::fmt::Display for InterceptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InterceptError::NotHeld => f.write_str("no request is held with that id"),
            InterceptError::NotEditable => {
                f.write_str("the request body was not buffered, so it cannot be edited")
            }
            InterceptError::Edit(e) => write!(f, "edited request does not parse: {e}"),
        }
    }
}

impl std::error::Error for InterceptError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            InterceptError::Edit(e) => Some(e),
            _ => None,
        }
    }
}

/// One request held in the queue.
struct Held {
    id: Ulid,
    host: String,
    method: String,
    path: String,
    size: usize,
    editable: bool,
    held_since: Instant,
    /// The original head bytes (the baseline for an edit diff).
    head: Bytes,
    /// The original body bytes, if buffered (the baseline for an edit diff).
    body: Option<Bytes>,
    /// Resolves the awaiting [`Intercept::on_request`] future.
    tx: oneshot::Sender<Decision>,
}

impl Held {
    fn summary(&self) -> HeldSummary {
        HeldSummary {
            id: self.id,
            host: self.host.clone(),
            method: self.method.clone(),
            path: self.path.clone(),
            size: self.size,
            editable: self.editable,
            held_since: self.held_since,
        }
    }
}

/// The request-intercept service.
pub struct Intercept {
    enabled: AtomicBool,
    rules: RwLock<InterceptRules>,
    /// Shared with the proxy's scope; kept for the API and future use.
    scope: Arc<RwLock<Compiled>>,
    /// The in-flight body budget held copies are charged to (shared with
    /// the proxy in the server wiring).
    budget: Arc<Budget>,
    /// Where the API records intercept actions (unused by the relay path).
    sink: Arc<dyn ExchangeSink>,
    queue: Mutex<Vec<Held>>,
    depth: usize,
    hold_timeout: Duration,
    parse: ParserConfig,
    version: watch::Sender<u64>,
}

impl std::fmt::Debug for Intercept {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Intercept")
            .field("enabled", &self.enabled.load(Ordering::Relaxed))
            .field("depth", &self.depth)
            .field("held", &self.queue.lock().map(|q| q.len()).unwrap_or(0))
            .finish_non_exhaustive()
    }
}

impl Intercept {
    /// Builds the service from the proxy limits, the shared scope, the
    /// starting rules, the shared budget and the sink.
    #[must_use]
    pub fn new(
        limits: &ProxyLimits,
        scope: Arc<RwLock<Compiled>>,
        rules: InterceptRules,
        budget: Arc<Budget>,
        sink: Arc<dyn ExchangeSink>,
    ) -> Intercept {
        let (version, _) = watch::channel(0);
        Intercept {
            enabled: AtomicBool::new(false),
            rules: RwLock::new(rules),
            scope,
            budget,
            sink,
            queue: Mutex::new(Vec::new()),
            depth: limits.intercept_queue.max(1),
            hold_timeout: limits.intercept_hold_timeout,
            parse: limits.parse,
            version,
        }
    }

    /// Whether intercept is holding requests.
    #[must_use]
    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }

    /// Turns intercept on or off. The caller audits the change.
    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::Release);
    }

    /// The current rules.
    #[must_use]
    pub fn rules(&self) -> InterceptRules {
        self.rules
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Replaces the rules.
    pub fn set_rules(&self, rules: InterceptRules) {
        *self.rules.write().unwrap_or_else(PoisonError::into_inner) = rules;
    }

    /// The scope shared with the proxy.
    #[must_use]
    pub fn scope(&self) -> &Arc<RwLock<Compiled>> {
        &self.scope
    }

    /// The budget shared with the proxy.
    #[must_use]
    pub fn budget(&self) -> &Arc<Budget> {
        &self.budget
    }

    /// The sink intercept actions are recorded to.
    #[must_use]
    pub fn sink(&self) -> &Arc<dyn ExchangeSink> {
        &self.sink
    }

    /// A snapshot of the held requests, oldest first.
    #[must_use]
    pub fn held(&self) -> Vec<HeldSummary> {
        self.queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(Held::summary)
            .collect()
    }

    /// The original head and body bytes of the held request `id`, cloned out
    /// from under the queue lock (the clone is the only work done while
    /// holding it; there is no `.await`). The body is `None` when the
    /// request's body was not buffered because it exceeded the hold limit.
    /// Returns `None` when no request is held with that id.
    #[must_use]
    pub fn held_bytes(&self, id: Ulid) -> Option<HeldBytes> {
        let queue = self.queue.lock().unwrap_or_else(PoisonError::into_inner);
        let held = queue.iter().find(|h| h.id == id)?;
        Some(HeldBytes {
            head: held.head.clone(),
            body: held.body.clone(),
        })
    }

    /// A receiver that observes the held-count version: it changes whenever
    /// the held set does.
    #[must_use]
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.version.subscribe()
    }

    /// Forwards the held request `id`, unchanged (`edited` is `None`) or
    /// with edited bytes.
    ///
    /// # Errors
    /// [`InterceptError::NotHeld`] when `id` is not held,
    /// [`InterceptError::NotEditable`] when edited bytes are given for a
    /// request whose body was not buffered, and [`InterceptError::Edit`]
    /// when the edited head does not parse. On any error the request stays
    /// held.
    pub fn forward(&self, id: Ulid, edited: Option<EditedBytes>) -> Result<(), InterceptError> {
        let mut queue = self.queue.lock().unwrap_or_else(PoisonError::into_inner);
        let pos = queue
            .iter()
            .position(|h| h.id == id)
            .ok_or(InterceptError::NotHeld)?;
        let decision = match edited {
            None => Decision::Forward,
            Some(edited) => {
                if !queue[pos].editable {
                    return Err(InterceptError::NotEditable);
                }
                let changes = self.changes_for(&queue[pos], &edited)?;
                Decision::Modify {
                    head: edited.head,
                    body: edited.body,
                    changes,
                }
            }
        };
        let held = queue.remove(pos);
        drop(queue);
        let _ = held.tx.send(decision);
        self.bump();
        Ok(())
    }

    /// Drops the held request `id`, closing its client connection.
    ///
    /// # Errors
    /// [`InterceptError::NotHeld`] when `id` is not held.
    pub fn drop(&self, id: Ulid) -> Result<(), InterceptError> {
        let held = self.take(id).ok_or(InterceptError::NotHeld)?;
        let _ = held.tx.send(Decision::Drop);
        self.bump();
        Ok(())
    }

    /// Forwards every held request unchanged.
    pub fn forward_all(&self) {
        let drained: Vec<Held> = {
            let mut queue = self.queue.lock().unwrap_or_else(PoisonError::into_inner);
            std::mem::take(&mut *queue)
        };
        if drained.is_empty() {
            return;
        }
        for held in drained {
            let _ = held.tx.send(Decision::Forward);
        }
        self.bump();
    }

    /// Validates the edited head and computes the recorded changes against
    /// the held request's original bytes.
    fn changes_for(
        &self,
        held: &Held,
        edited: &EditedBytes,
    ) -> Result<Vec<Change>, InterceptError> {
        validate_request_head(&self.parse, &edited.head).map_err(InterceptError::Edit)?;
        let mut changes = Vec::new();
        if edited.head != held.head {
            changes.push(Change::new(
                ChangeOp::Replace,
                whole_span(edited.head.len()),
                held.head.clone(),
                edited.head.clone(),
                ChangeReason::InterceptEdit,
            ));
        }
        let old_body = held.body.clone().unwrap_or_default();
        let new_body = edited.body.clone().unwrap_or_default();
        if new_body != old_body {
            changes.push(Change::new(
                ChangeOp::Replace,
                whole_span(new_body.len()),
                old_body,
                new_body,
                ChangeReason::InterceptEdit,
            ));
        }
        Ok(changes)
    }

    /// Removes and returns the held request `id`.
    fn take(&self, id: Ulid) -> Option<Held> {
        let mut queue = self.queue.lock().unwrap_or_else(PoisonError::into_inner);
        let pos = queue.iter().position(|h| h.id == id)?;
        Some(queue.remove(pos))
    }

    /// Bumps the held-count version.
    fn bump(&self) {
        self.version.send_modify(|v| *v = v.wrapping_add(1));
    }
}

impl Handler for Intercept {
    fn holds(&self, head: &RawMessage, conn: &ConnectionInfo, scope_state: ScopeState) -> bool {
        if !self.enabled() {
            return false;
        }
        let _ = conn;
        let path = head
            .head()
            .start_target_range()
            .map(|r| String::from_utf8_lossy(&head.bytes()[r]).into_owned())
            .unwrap_or_default();
        // Decide under the read guard so the extension list is never cloned.
        let r = self.rules.read().unwrap_or_else(PoisonError::into_inner);
        if r.skip_out_of_scope && scope_state == ScopeState::Out {
            return false;
        }
        !skipped_by_extension(&path, &r.skip_extensions)
    }

    fn on_request<'a>(&'a self, req: HeldRequest<'a>) -> BoxFuture<'a, Decision> {
        Box::pin(async move {
            if !self.enabled() {
                return Decision::Forward;
            }
            let (method, host, path) = summarize(&req);
            // Decide skips under the read guard (no clone of the extension
            // list); keep only the two Copy settings needed after the await.
            let (overflow, on_timeout) = {
                let r = self.rules.read().unwrap_or_else(PoisonError::into_inner);
                if (r.skip_out_of_scope && req.scope_state == ScopeState::Out)
                    || skipped_by_extension(&path, &r.skip_extensions)
                {
                    return Decision::Forward;
                }
                (r.overflow, r.on_timeout)
            };
            let id = Ulid::new();
            let (tx, rx) = oneshot::channel();
            let held = Held {
                id,
                host,
                method,
                path,
                size: req.body.map_or(0, Bytes::len),
                editable: req.editable,
                held_since: Instant::now(),
                head: req.head.bytes().clone(),
                body: req.body.cloned(),
                tx,
            };
            {
                let mut queue = self.queue.lock().unwrap_or_else(PoisonError::into_inner);
                if queue.len() >= self.depth {
                    match overflow {
                        Overflow::ForwardNewest => return Decision::Forward,
                        Overflow::ForwardOldest => {
                            if !queue.is_empty() {
                                let oldest = queue.remove(0);
                                let _ = oldest.tx.send(Decision::Forward);
                            }
                        }
                    }
                }
                queue.push(held);
            }
            self.bump();
            match tokio::time::timeout(self.hold_timeout, rx).await {
                // Forward/drop/modify arrived; the entry was already removed
                // by the API call that sent it.
                Ok(Ok(decision)) => decision,
                // The sender was dropped without a decision: forward safely.
                Ok(Err(_)) => Decision::Forward,
                Err(_) => {
                    // The hold timed out; drop our own entry and act per rule.
                    if self.take(id).is_some() {
                        self.bump();
                    }
                    match on_timeout {
                        OnTimeout::Forward => Decision::Forward,
                        OnTimeout::Respond504 => Decision::Respond504,
                    }
                }
            }
        })
    }

    fn on_response_head(&self, _resp: &RawMessage, _req: &RawMessage) -> Decision {
        Decision::Forward
    }
}

/// The method, host and path of a held request, for the summary and rules.
fn summarize(req: &HeldRequest<'_>) -> (String, String, String) {
    let head = req.head;
    let buf = head.bytes();
    let method = match head.head().start() {
        StartLine::Request { method, .. } => {
            String::from_utf8_lossy(&buf[method.range()]).into_owned()
        }
        _ => String::new(),
    };
    let path = head
        .head()
        .start_target_range()
        .map(|r| String::from_utf8_lossy(&buf[r]).into_owned())
        .unwrap_or_default();
    let host = head
        .head()
        .header(buf, "Host")
        .map(|s| String::from_utf8_lossy(&buf[s.range()]).trim().to_owned())
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| req.conn.target_addr().to_owned());
    (method, host, path)
}

/// Whether `path`'s extension is in `exts` (case-insensitive, after
/// stripping any query string).
fn skipped_by_extension(path: &str, exts: &[String]) -> bool {
    let no_query = path.split(['?', '#']).next().unwrap_or(path);
    let last = no_query.rsplit('/').next().unwrap_or(no_query);
    let Some(dot) = last.rfind('.') else {
        return false;
    };
    let ext = &last[dot + 1..];
    !ext.is_empty() && exts.iter().any(|e| e.eq_ignore_ascii_case(ext))
}

/// An after-coordinate span `[0, len)` for a whole-head or whole-body
/// replacement (a precise byte diff is not available from `EditedBytes`).
fn whole_span(len: usize) -> Span {
    Span::new(0, u32::try_from(len).unwrap_or(u32::MAX))
}

/// Confirms `bytes` parse as a complete request head.
fn validate_request_head(cfg: &ParserConfig, bytes: &[u8]) -> Result<(), EditError> {
    let mut parser = Parser::request(cfg);
    match parser.feed(bytes) {
        Ok(step) if matches!(step.event, Progress::Head(_)) => Ok(()),
        Ok(_) => Err(EditError::Reparse(
            parser.feed_eof().err().unwrap_or(ParseError::UnexpectedEof),
        )),
        Err(e) => Err(EditError::Reparse(e)),
    }
}
