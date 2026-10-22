//! The writer thread: batches exchanges into `SQLite` and packs.
//!
//! Per batch, in this order: `BEGIN IMMEDIATE`; for each exchange the bodies
//! (`BodyStore::put`, appending to packs) and the `exchanges` row; advance
//! `meta.last_committed_ulid`; a pending gap marker; fsync the pack; record
//! each touched pack's `committed_len`; `COMMIT`; then broadcast
//! `Committed`. Nothing is announced as committed before `COMMIT` returned,
//! and `COMMIT` never happens before the pack bytes it references are synced.

use crate::bodies::{BodyLocator, BodySide, BodyStore};
use crate::error::StoreError;
use crate::journal::{Entry, Journal};
use crate::packs::PackWriter;
use crate::sink::{Msg, Shared, Stop};
use crate::store::StoreConfig;
use crate::Project;
use crosswire_model::{
    Body, Event, Exchange, ExchangeState, Head, Marker, MarkerKind, Message, StartLineSpans,
};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, PoisonError};
use std::time::{Duration, Instant};
use ulid::Ulid;

/// An exchange to commit and, if it came from the journal, its file.
struct Item {
    ex: Exchange,
    journal: Option<Entry>,
}

/// What a successful commit produced.
struct Committed {
    ids: Vec<Ulid>,
    marker: Option<Marker>,
}

/// Owns the project (and so its one writer connection) on the writer thread.
pub(crate) struct Writer {
    project: Project,
    bodies: Option<BodyStore>,
    shared: Arc<Shared>,
    cfg: StoreConfig,
    last_checkpoint: Instant,
}

impl Writer {
    pub(crate) fn new(project: Project, shared: Arc<Shared>, cfg: StoreConfig) -> Writer {
        Writer {
            project,
            bodies: None,
            shared,
            cfg,
            last_checkpoint: Instant::now(),
        }
    }

    /// Commits every journal file, oldest first.
    pub(crate) fn drain_journal(&mut self) {
        while self.drain_journal_once() > 0 {}
    }

    /// Inserts the `Gap` marker for an unclean previous run.
    pub(crate) fn mark_unclean_restart(&mut self) -> Result<(), StoreError> {
        let tx = self.project.writer_mut().transaction()?;
        let marker = gap_marker(&tx, "unclean shutdown")?;
        tx.commit()?;
        self.shared.events.emit(|seq| Event::Marker { marker, seq });
        Ok(())
    }

    /// Records `clean=0`: from here on a crash is an unclean shutdown.
    pub(crate) fn mark_dirty(&self) -> Result<(), StoreError> {
        self.project.mark_dirty()
    }

    /// The thread body. Returns the project once stopped.
    pub(crate) fn run(mut self, rx: &Receiver<Msg>) -> Project {
        let mut stop = None;
        while stop.is_none() {
            let pause = self.cfg.test_pause.clone();
            let guard = pause
                .as_ref()
                .map(|m| m.lock().unwrap_or_else(PoisonError::into_inner));
            let batch = self.collect(rx, &mut stop);
            self.commit(batch);
            if self.shared.queued.load(Ordering::SeqCst) * 4 < self.cfg.queue_max {
                self.drain_journal_once();
            }
            self.maybe_checkpoint();
            drop(guard);
            if pause.is_some() {
                // Let a test that waits for the pause lock take it.
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        // Everything was enqueued before the stop message; take what is left.
        loop {
            let mut batch = Vec::new();
            while batch.len() < self.cfg.batch_max.max(1) {
                match rx.try_recv() {
                    Ok(Msg::Exchange(ex, size)) => {
                        self.shared.release(size);
                        batch.push(Item {
                            ex: *ex,
                            journal: None,
                        });
                    }
                    Ok(Msg::Stop(_)) => {}
                    Err(_) => break,
                }
            }
            if batch.is_empty() {
                break;
            }
            self.commit(batch);
        }
        self.drain_journal();
        // Records a gap that is still pending.
        self.commit(Vec::new());
        if stop == Some(Stop::Clean) {
            self.checkpoint("TRUNCATE");
        }
        self.project
    }

    /// Waits up to `batch_interval` for a first exchange, then collects until
    /// `batch_max` or `batch_interval` after the first.
    fn collect(&self, rx: &Receiver<Msg>, stop: &mut Option<Stop>) -> Vec<Item> {
        // Clamped so `Instant + interval` cannot overflow.
        let interval = self.cfg.batch_interval.min(Duration::from_secs(86_400));
        let mut batch = Vec::new();
        let mut next = rx.recv_timeout(interval);
        let deadline = Instant::now() + interval;
        loop {
            match next {
                Ok(Msg::Exchange(ex, size)) => {
                    self.shared.release(size);
                    batch.push(Item {
                        ex: *ex,
                        journal: None,
                    });
                }
                Ok(Msg::Stop(s)) => {
                    *stop = Some(s);
                    break;
                }
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => {
                    *stop = Some(Stop::Abandon);
                    break;
                }
            }
            if batch.len() >= self.cfg.batch_max.max(1) {
                break;
            }
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            next = rx.recv_timeout(deadline - now);
        }
        batch
    }

    /// Commits up to `batch_max` journal files; returns how many files left
    /// the journal (committed or quarantined).
    fn drain_journal_once(&mut self) -> usize {
        let entries = match self.shared.journal.oldest(self.cfg.batch_max.max(1)) {
            Ok(e) => e,
            Err(e) => {
                self.shared
                    .events
                    .error(format!("cannot list the journal: {e}"));
                return 0;
            }
        };
        let mut removed = 0;
        let mut items = Vec::new();
        for entry in entries {
            match Journal::read(&entry) {
                Ok(ex) => items.push(Item {
                    ex,
                    journal: Some(entry),
                }),
                Err(why) => {
                    if self.quarantine_file(&entry, &why) {
                        removed += 1;
                    }
                }
            }
        }
        removed + self.commit(items)
    }

    /// Moves an unusable journal file to `journal/bad/` and reports it.
    fn quarantine_file(&self, entry: &Entry, why: &str) -> bool {
        let name = entry.path.file_name().unwrap_or_default().to_string_lossy();
        match self.shared.journal.quarantine(entry) {
            Ok(()) => {
                self.shared.events.error(format!(
                    "journal file {name} is unusable ({why}); moved to journal/bad/"
                ));
                true
            }
            Err(e) => {
                self.shared.events.error(format!(
                    "journal file {name} is unusable ({why}) and could not be moved: {e}"
                ));
                false
            }
        }
    }

    /// Commits `items` as one transaction, or one by one if that fails.
    /// Returns how many journal files were removed.
    fn commit(&mut self, items: Vec<Item>) -> usize {
        let gap = self.shared.gap_pending.swap(false, Ordering::SeqCst);
        if items.is_empty() && !gap {
            return 0;
        }
        match self.try_commit(&items, gap) {
            Ok(done) => self.after_commit(items, done),
            Err(e) => {
                // The pack writer may have appended past what the database now
                // knows, or rotated in the rolled-back transaction: reopen it.
                self.bodies = None;
                if gap {
                    self.shared.gap_pending.store(true, Ordering::SeqCst);
                }
                if items.len() > 1 {
                    self.shared.events.error(format!(
                        "batch of {} failed ({e}); retrying one by one",
                        items.len()
                    ));
                    items.into_iter().map(|i| self.commit(vec![i])).sum()
                } else if let Some(item) = items.into_iter().next() {
                    usize::from(self.give_up(item, &e))
                } else {
                    self.shared
                        .events
                        .error(format!("cannot write the gap marker: {e}"));
                    0
                }
            }
        }
    }

    /// An exchange that cannot be committed is kept in `journal/bad/` and
    /// counted as a loss (a gap marker follows).
    fn give_up(&mut self, item: Item, e: &StoreError) -> bool {
        let id = item.ex.id();
        self.shared.gap_pending.store(true, Ordering::SeqCst);
        if let Some(entry) = &item.journal {
            return self.quarantine_file(entry, &format!("commit failed: {e}"));
        }
        let kept = match self.shared.journal.quarantine_exchange(&item.ex) {
            Ok(()) => "kept in journal/bad/".to_owned(),
            Err(e2) => format!("and could not be kept: {e2}"),
        };
        self.shared.events.error(format!(
            "exchange {id} could not be committed ({e}); {kept}"
        ));
        false
    }

    /// After `COMMIT`: deletes the batch's journal files, then broadcasts, so a
    /// subscriber that saw `Committed` never finds the file still there.
    fn after_commit(&mut self, items: Vec<Item>, done: Committed) -> usize {
        let Committed { ids, marker } = done;
        let mut removed = 0;
        for entry in items.into_iter().filter_map(|i| i.journal) {
            match self.shared.journal.remove(&entry) {
                Ok(()) => removed += 1,
                // Replay is idempotent: a leftover file is skipped next time.
                Err(e) => self.shared.events.error(format!(
                    "cannot delete committed journal file {}: {e}",
                    entry.path.display()
                )),
            }
        }
        if !ids.is_empty() {
            self.shared.events.emit(|seq| Event::Committed { ids, seq });
        }
        if let Some(marker) = marker {
            self.shared.events.emit(|seq| Event::Marker { marker, seq });
        }
        removed
    }

    fn try_commit(&mut self, items: &[Item], gap: bool) -> Result<Committed, StoreError> {
        if self.bodies.is_none() {
            self.bodies = Some(BodyStore::new(PackWriter::open(&mut self.project)?));
        }
        let Writer {
            project, bodies, ..
        } = self;
        let Some(bodies) = bodies.as_mut() else {
            return Err(StoreError::Closed);
        };
        // (1) BEGIN IMMEDIATE.
        let tx = project
            .writer_mut()
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        // Packs appended to in this batch, and the end of the last record in each.
        let mut touched: BTreeMap<u32, u64> = BTreeMap::new();
        let mut ids = Vec::with_capacity(items.len());
        // (5) A pending gap starts at the last id committed before this batch.
        let marker = if gap {
            Some(gap_marker(
                &tx,
                "exchanges were dropped: the queue and journal were full or unwritable",
            )?)
        } else {
            None
        };
        for item in items {
            let id = item.ex.id();
            let exists = tx
                .query_row(
                    "SELECT 1 FROM exchanges WHERE ulid = ?1",
                    [id.to_string()],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            // (2) Bodies and the row; a replayed or repeated id is already stored.
            if !exists {
                insert_exchange(&tx, bodies, &item.ex, &mut touched)?;
            }
            ids.push(id);
        }
        // (3) Advance last_committed_ulid (ULID order; never backwards).
        if let Some(max) = ids.iter().max() {
            tx.execute(
                "UPDATE meta SET value = ?1 WHERE key = 'last_committed_ulid' AND value < ?1",
                [max.to_string()],
            )?;
        }
        if !touched.is_empty() {
            // (4) The pack bytes are durable before anything references them.
            bodies.fsync()?;
            // (5) Trust them from now on.
            for (pack, len) in &touched {
                Project::set_pack_committed_len(&tx, *pack, *len)?;
            }
        }
        // (6) COMMIT.
        tx.commit()?;
        Ok(Committed { ids, marker })
    }

    fn maybe_checkpoint(&mut self) {
        let wal = self
            .project
            .dir()
            .join(format!("{}-wal", crate::project::DB_FILE));
        let wal_len = std::fs::metadata(&wal).map_or(0, |m| m.len());
        if self.last_checkpoint.elapsed() >= self.cfg.checkpoint_interval
            || wal_len >= self.cfg.checkpoint_bytes
        {
            self.checkpoint("PASSIVE");
            // Make the WAL durable even if readers kept the checkpoint from
            // copying it: this bounds a power loss to one interval.
            if let Err(e) = std::fs::File::open(&wal).and_then(|f| f.sync_all()) {
                if e.kind() != std::io::ErrorKind::NotFound {
                    self.shared
                        .events
                        .error(format!("cannot sync the WAL: {e}"));
                }
            }
        }
    }

    fn checkpoint(&mut self, mode: &str) {
        self.last_checkpoint = Instant::now();
        let sql = format!("PRAGMA wal_checkpoint({mode})");
        let res: rusqlite::Result<(i64, i64, i64)> =
            self.project
                .writer_mut()
                .query_row(&sql, [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)));
        if let Err(e) = res {
            self.shared
                .events
                .error(format!("wal_checkpoint({mode}) failed: {e}"));
        }
    }
}

/// Inserts a `Gap` marker from `meta.last_committed_ulid` and returns it.
fn gap_marker(tx: &Transaction<'_>, detail: &str) -> Result<Marker, StoreError> {
    let last: String = tx.query_row(
        "SELECT value FROM meta WHERE key = 'last_committed_ulid'",
        [],
        |r| r.get(0),
    )?;
    let mut marker = Marker::new(MarkerKind::Gap, detail);
    if let Ok(from) = Ulid::from_string(&last) {
        marker = marker.with_from(from);
    }
    tx.execute(
        "INSERT INTO markers (ulid, kind, from_ulid, to_ulid, detail) VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![
            marker.id().to_string(),
            wire_name(&marker.kind()),
            marker.from_ulid().map(|u| u.to_string()),
            marker.to_ulid().map(|u| u.to_string()),
            marker.detail(),
        ],
    )?;
    Ok(marker)
}

/// Stores one exchange's bodies and row inside `tx`.
fn insert_exchange(
    tx: &Transaction<'_>,
    bodies: &mut BodyStore,
    ex: &Exchange,
    touched: &mut BTreeMap<u32, u64>,
) -> Result<(), StoreError> {
    let id = ex.id();
    let mut json = serde_json::to_value(ex)?;
    set_field(
        &mut json,
        "/state",
        serde_json::to_value(ExchangeState::Committed)?,
    )?;
    let sides = [
        (BodySide::Req, Some(ex.request()), "/request/body"),
        (BodySide::Resp, ex.response(), "/response/body"),
    ];
    for (side, msg, key) in sides {
        let Some(Body::Inline { bytes }) = msg.map(Message::body) else {
            continue;
        };
        let hash: [u8; 32] = match bodies.put(tx, id, side, bytes)? {
            BodyLocator::Inline { .. } => Sha256::digest(bytes).into(),
            BodyLocator::Pack { blob, deduped } => {
                if !deduped {
                    let end = touched.entry(bodies.pack_id()).or_insert(0);
                    *end = (*end).max(bodies.committed_len());
                }
                blob.hash
            }
        };
        // The JSON keeps a reference; the bytes live in `bodies` and packs.
        let blob = serde_json::to_value(Body::Blob {
            hash,
            len: bytes.len() as u64,
            truncated_at: None,
            capture_dropped: false,
        })?;
        set_field(&mut json, key, blob)?;
    }
    let (method, path) = request_line(ex.request());
    let host = Some(ex.request().connection().target_addr()).filter(|h| !h.is_empty());
    let status = ex.response().and_then(status_code);
    let resp_len = ex.response().map(|m| match m.body() {
        Body::Inline { bytes } => bytes.len() as u64,
        Body::Blob { len, .. } => *len,
        _ => 0,
    });
    let resp_len = resp_len.map(|l| i64::try_from(l).unwrap_or(i64::MAX));
    let tags = if ex.tags().is_empty() {
        String::new()
    } else {
        serde_json::to_string(ex.tags())?
    };
    let created = i64::try_from(id.timestamp_ms()).unwrap_or(i64::MAX);
    tx.execute(
        "INSERT INTO exchanges (ulid, namespace, state, scope_state, created_at, json, \
         req_method, req_host, req_path, resp_status, resp_len, tags, notes) \
         VALUES (?1, ?2, 'committed', ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        rusqlite::params![
            id.to_string(),
            wire_name(&ex.namespace().kind()),
            wire_name(&ex.scope_state()),
            created,
            serde_json::to_string(&json)?,
            method,
            host,
            path,
            status,
            resp_len,
            tags,
            ex.notes(),
        ],
    )?;
    Ok(())
}

/// Replaces the value at `pointer` (which must exist) without indexing panics.
fn set_field(
    json: &mut serde_json::Value,
    pointer: &str,
    value: serde_json::Value,
) -> Result<(), StoreError> {
    let slot = json
        .pointer_mut(pointer)
        .ok_or_else(|| StoreError::Corrupt(format!("exchange json has no {pointer}")))?;
    *slot = value;
    Ok(())
}

/// The kebab-case wire name of a unit enum value.
fn wire_name<T: Serialize>(v: &T) -> String {
    match serde_json::to_value(v) {
        Ok(serde_json::Value::String(s)) => s,
        _ => "unknown".to_owned(),
    }
}

/// Method and target of an HTTP/1 request head.
fn request_line(m: &Message) -> (Option<String>, Option<String>) {
    if let Head::Http1Raw { bytes, spans, .. } = m.head() {
        if let StartLineSpans::Request { method, target, .. } = spans.start_line() {
            let text = |r: std::ops::Range<usize>| {
                bytes
                    .get(r)
                    .map(|b| String::from_utf8_lossy(b).into_owned())
            };
            return (text(method.range()), text(target.range()));
        }
    }
    (None, None)
}

/// The status code of an HTTP/1 response head.
fn status_code(m: &Message) -> Option<i64> {
    let Head::Http1Raw { bytes, spans, .. } = m.head() else {
        return None;
    };
    let StartLineSpans::Response { status, .. } = spans.start_line() else {
        return None;
    };
    std::str::from_utf8(bytes.get(status.range())?)
        .ok()?
        .parse::<u16>()
        .ok()
        .map(i64::from)
}
