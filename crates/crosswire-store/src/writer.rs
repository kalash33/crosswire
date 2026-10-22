//! The writer thread: batches exchanges into `SQLite` and packs.
//!
//! Per batch, in this order: `BEGIN IMMEDIATE`; a pending gap marker; for
//! each exchange the bodies (`BodyStore::put`, appending to packs) and the
//! `exchanges` row; advance `meta.last_committed_ulid` and
//! `meta.last_commit_watermark`; fsync the pack; record each touched pack's
//! `committed_len`; `COMMIT`; then (if journal files were committed) fsync
//! the WAL and delete them; then broadcast `Committed`. Nothing is announced
//! as committed before `COMMIT` returned, and `COMMIT` never happens before
//! the pack bytes it references are synced.
//!
//! Failures are split in two. A failure caused by one exchange (an oversized
//! body, a JSON or constraint error) sets that exchange aside in
//! `journal/bad/` and commits the rest. Any other failure is treated as the
//! database being unavailable (disk full, I/O error): queued exchanges are
//! written back to `journal/` (or held in memory if that fails too), journal
//! files stay where they are, one `Error` is sent per failure streak, and the
//! writer retries with a backoff from 250 ms doubling to 30 s.

use crate::bodies::{BodyLocator, BodySide, BodyStore};
use crate::error::StoreError;
use crate::journal::{Entry, Journal};
use crate::packs::PackWriter;
use crate::sink::{slack_ulid, Msg, Shared, Stop};
use crate::store::StoreConfig;
use crate::Project;
use crosswire_model::{
    Body, Event, Exchange, ExchangeState, Head, Marker, MarkerKind, Message, StartLineSpans,
};
use rusqlite::{ErrorCode, OptionalExtension, Transaction, TransactionBehavior};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, PoisonError};
use std::time::{Duration, Instant};
use ulid::Ulid;

const BACKOFF_MIN: Duration = Duration::from_millis(250);
const BACKOFF_MAX: Duration = Duration::from_secs(30);

/// An exchange to commit, and where it came from.
struct Item {
    ex: Exchange,
    /// The journal file it was read from.
    journal: Option<Entry>,
    /// The queue bytes it reserved (it came from the queue and is in memory only).
    reserved: Option<usize>,
}

impl Item {
    fn queued(ex: Exchange, size: usize) -> Item {
        Item {
            ex,
            journal: None,
            reserved: Some(size),
        }
    }
}

/// What a successful commit produced.
struct Committed {
    ids: Vec<Ulid>,
    marker: Option<Marker>,
}

/// Why a commit failed.
enum CommitError {
    /// Exchange `items[.0]` cannot be stored; the others may be fine.
    Item(usize, StoreError),
    /// The database or disk failed; nothing in the batch is at fault.
    Db(StoreError),
}

impl From<StoreError> for CommitError {
    fn from(e: StoreError) -> CommitError {
        CommitError::Db(e)
    }
}

impl From<rusqlite::Error> for CommitError {
    fn from(e: rusqlite::Error) -> CommitError {
        CommitError::Db(e.into())
    }
}

/// Whether an error from storing one exchange is that exchange's fault.
fn per_exchange(e: &StoreError) -> bool {
    match e {
        StoreError::Json(_) => true,
        StoreError::Io(io) => io.kind() == std::io::ErrorKind::InvalidInput,
        StoreError::Sqlite(rusqlite::Error::SqliteFailure(f, _)) => matches!(
            f.code,
            ErrorCode::ConstraintViolation | ErrorCode::TooBig | ErrorCode::TypeMismatch
        ),
        StoreError::Sqlite(rusqlite::Error::ToSqlConversionFailure(_)) => true,
        _ => false,
    }
}

/// Marks the writer dead if its thread unwinds.
struct DeathWatch(Arc<Shared>);

impl Drop for DeathWatch {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.0.writer_died("the writer thread panicked");
        }
    }
}

/// Owns the project (and so its one writer connection) on the writer thread.
pub(crate) struct Writer {
    project: Project,
    bodies: Option<BodyStore>,
    shared: Arc<Shared>,
    cfg: StoreConfig,
    last_checkpoint: Instant,
    /// Queued exchanges that could be neither committed nor written back.
    held: Vec<Item>,
    /// The current backoff delay and when to try again, while the database fails.
    backoff: Option<(Duration, Instant)>,
    /// In a database failure streak (its one `Error` was sent).
    failing: bool,
    /// Journal files whose quarantine failed; skipped from then on.
    stuck: HashSet<PathBuf>,
}

impl Writer {
    pub(crate) fn new(project: Project, shared: Arc<Shared>, cfg: StoreConfig) -> Writer {
        Writer {
            project,
            bodies: None,
            shared,
            cfg,
            last_checkpoint: Instant::now(),
            held: Vec::new(),
            backoff: None,
            failing: false,
            stuck: HashSet::new(),
        }
    }

    /// Commits every journal file, oldest first; returns how many left the
    /// journal. Stops early while the database is failing.
    pub(crate) fn drain_journal(&mut self) -> usize {
        let mut total = 0;
        loop {
            let n = self.drain_journal_once();
            total += n;
            if n == 0 || self.backing_off() {
                return total;
            }
        }
    }

    /// Inserts the `Gap` marker for an unclean previous run: from the last
    /// commit's watermark (read before anything else is committed) to now.
    pub(crate) fn mark_unclean_restart(&mut self) -> Result<Marker, StoreError> {
        let tx = self.project.writer_mut().transaction()?;
        let from: Option<String> = tx
            .query_row(
                "SELECT value FROM meta WHERE key = 'last_commit_watermark'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        let from = from.and_then(|f| Ulid::from_string(&f).ok());
        let marker = insert_gap(&tx, from, "unclean shutdown")?;
        tx.commit()?;
        let event = marker.clone();
        self.shared
            .events
            .emit(|seq| Event::Marker { marker: event, seq });
        Ok(marker)
    }

    /// Records `clean=0`: from here on a crash is an unclean shutdown.
    pub(crate) fn mark_dirty(&self) -> Result<(), StoreError> {
        self.project.mark_dirty()
    }

    /// The thread body. Returns the project once stopped.
    pub(crate) fn run(mut self, rx: &Receiver<Msg>) -> Project {
        let _watch = DeathWatch(Arc::clone(&self.shared));
        let mut stop = None;
        while stop.is_none() {
            if self.wait_backoff(rx, &mut stop) {
                continue;
            }
            let pause = self.cfg.test_pause.clone();
            let guard = pause
                .as_ref()
                .map(|m| m.lock().unwrap_or_else(PoisonError::into_inner));
            let batch = if self.held.is_empty() {
                self.collect(rx, &mut stop)
            } else {
                std::mem::take(&mut self.held)
            };
            #[cfg(test)]
            if !batch.is_empty()
                && self
                    .cfg
                    .hooks
                    .panic_next_batch
                    .swap(false, Ordering::SeqCst)
            {
                panic!("injected writer panic");
            }
            self.commit(batch);
            // On a stop the journal is `finish`'s business (and `Drop` skips it).
            if stop.is_none()
                && !self.backing_off()
                && self.shared.queued.load(Ordering::SeqCst) * 4 < self.cfg.queue_max
            {
                self.drain_journal_once();
            }
            self.maybe_checkpoint();
            drop(guard);
            if pause.is_some() {
                // Let a test that waits for the pause lock take it.
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        self.finish(rx, stop == Some(Stop::Clean));
        self.project
    }

    /// The shutdown path. Everything was enqueued before the stop message.
    fn finish(&mut self, rx: &Receiver<Msg>, clean: bool) {
        // One attempt each from here: no backoff waits during shutdown.
        self.backoff = None;
        let mut batch = std::mem::take(&mut self.held);
        while let Ok(msg) = rx.try_recv() {
            if let Msg::Exchange(ex, size) = msg {
                batch.push(Item::queued(*ex, size));
            }
        }
        for chunk in chunked(batch, self.cfg.batch_max.max(1)) {
            self.backoff = None;
            self.commit(chunk);
        }
        // `close` drains the journal; `Drop` leaves it for the next open,
        // so dropping a store never waits on a large journal.
        if clean {
            self.backoff = None;
            self.drain_journal();
        }
        self.backoff = None;
        self.commit(Vec::new());
        // Whatever is still only in memory is lost: record it.
        for item in std::mem::take(&mut self.held) {
            if self.shared.journal.write_back(&item.ex).is_err() {
                self.shared
                    .dropped(item.ex.id(), "exchanges could not be stored at shutdown");
            }
            if let Some(size) = item.reserved {
                self.shared.release(item.ex.id(), size);
            }
        }
        if clean {
            self.checkpoint("TRUNCATE");
        }
    }

    /// Whether a database failure's retry time is still ahead.
    fn backing_off(&self) -> bool {
        self.backoff.is_some_and(|(_, at)| Instant::now() < at)
    }

    /// While backing off, waits (in steps, so a stop is seen) and keeps
    /// receiving into `held`. Returns true if the caller should loop again.
    fn wait_backoff(&mut self, rx: &Receiver<Msg>, stop: &mut Option<Stop>) -> bool {
        let Some((_, at)) = self.backoff else {
            return false;
        };
        let now = Instant::now();
        if now >= at {
            return false;
        }
        match rx.recv_timeout((at - now).min(Duration::from_millis(50))) {
            Ok(Msg::Exchange(ex, size)) => self.held.push(Item::queued(*ex, size)),
            Ok(Msg::Stop(s)) => *stop = Some(s),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => *stop = Some(Stop::Abandon),
        }
        true
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
                Ok(Msg::Exchange(ex, size)) => batch.push(Item::queued(*ex, size)),
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

    /// Commits up to `batch_max` journal files, and at most `queue_bytes_max`
    /// of them (always at least one); returns how many files left the
    /// journal (committed or quarantined).
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
        let mut bytes = 0u64;
        for entry in entries {
            if self.stuck.contains(&entry.path) {
                continue;
            }
            if !items.is_empty()
                && bytes.saturating_add(entry.len) > self.cfg.queue_bytes_max as u64
            {
                break;
            }
            bytes = bytes.saturating_add(entry.len);
            match Journal::read(&entry) {
                Ok(ex) => items.push(Item {
                    ex,
                    journal: Some(entry),
                    reserved: None,
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

    /// Moves an unusable journal file to `journal/bad/` and reports it. A
    /// file that cannot be moved is skipped from then on.
    fn quarantine_file(&mut self, entry: &Entry, why: &str) -> bool {
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
                    "journal file {name} is unusable ({why}) and could not be moved \
                     ({e}); skipped until the next open"
                ));
                self.stuck.insert(entry.path.clone());
                false
            }
        }
    }

    /// Commits `items` as one transaction. An exchange at fault is set aside
    /// and the rest retried; a database failure starts the backoff. Returns
    /// how many journal files were removed.
    fn commit(&mut self, mut items: Vec<Item>) -> usize {
        let mut removed = 0;
        loop {
            let gap = self.shared.take_gap();
            if items.is_empty() && gap.is_none() {
                return removed;
            }
            match self.try_commit(&items, gap) {
                Ok(done) => {
                    self.backoff = None;
                    self.failing = false;
                    return removed + self.after_commit(items, done);
                }
                Err(err) => {
                    // The pack writer may have appended past what the database
                    // knows, or rotated in the rolled-back transaction: reopen it.
                    self.bodies = None;
                    if let Some(g) = gap {
                        self.shared.restore_gap(g);
                    }
                    match err {
                        CommitError::Item(i, e) if i < items.len() => {
                            let item = items.remove(i);
                            removed += usize::from(self.give_up(item, &e));
                        }
                        CommitError::Item(_, e) | CommitError::Db(e) => {
                            self.db_failure(items, &e);
                            return removed;
                        }
                    }
                }
            }
        }
    }

    /// The database is unavailable: keep every exchange and back off.
    fn db_failure(&mut self, items: Vec<Item>, e: &StoreError) {
        if !self.failing {
            self.failing = true;
            self.shared.events.error(format!(
                "cannot write to the project ({e}); exchanges are kept in journal/ \
                 and retried"
            ));
        }
        let delay = match self.backoff {
            None => BACKOFF_MIN,
            Some((d, _)) => (d * 2).min(BACKOFF_MAX),
        };
        self.backoff = Some((delay, Instant::now() + delay));
        for item in items {
            // Journal items are durable already: their files stay.
            let Some(size) = item.reserved else {
                continue;
            };
            if self.shared.journal.write_back(&item.ex).is_ok() {
                self.shared.release(item.ex.id(), size);
            } else {
                self.held.push(item);
            }
        }
    }

    /// An exchange that cannot be stored is kept in `journal/bad/` and
    /// counted as a loss (a gap marker follows).
    fn give_up(&mut self, item: Item, e: &StoreError) -> bool {
        let id = item.ex.id();
        if let Some(entry) = &item.journal {
            let moved = self.quarantine_file(entry, &format!("cannot be stored: {e}"));
            // Recorded once: a file that stays stuck is not retried.
            self.shared
                .dropped(id, "an exchange could not be stored; see journal/bad/");
            return moved;
        }
        let kept = match self.shared.journal.quarantine_exchange(&item.ex) {
            Ok(()) => "kept in journal/bad/".to_owned(),
            Err(e2) => format!("and could not be kept: {e2}"),
        };
        if let Some(size) = item.reserved {
            self.shared.release(id, size);
        }
        self.shared
            .events
            .error(format!("exchange {id} cannot be stored ({e}); {kept}"));
        self.shared
            .dropped(id, "an exchange could not be stored; see journal/bad/");
        false
    }

    /// After `COMMIT`: releases queue reservations, makes the commit durable
    /// before deleting the batch's journal files (so a spilled exchange is
    /// never only in an unsynced WAL), then broadcasts.
    fn after_commit(&mut self, items: Vec<Item>, done: Committed) -> usize {
        let Committed { ids, marker } = done;
        let mut journaled = Vec::new();
        for item in items {
            if let Some(size) = item.reserved {
                self.shared.release(item.ex.id(), size);
            }
            if let Some(entry) = item.journal {
                journaled.push(entry);
            }
        }
        let mut removed = 0;
        if !journaled.is_empty() {
            match self.sync_wal() {
                Ok(()) => {
                    for entry in journaled {
                        match self.shared.journal.remove(&entry) {
                            Ok(()) => removed += 1,
                            // Replay is idempotent: a leftover file is skipped next time.
                            Err(e) => self.shared.events.error(format!(
                                "cannot delete committed journal file {}: {e}",
                                entry.path.display()
                            )),
                        }
                    }
                }
                Err(e) => self.shared.events.error(format!(
                    "cannot sync the WAL ({e}); committed journal files are kept"
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

    fn try_commit(&mut self, items: &[Item], gap: Option<Ulid>) -> Result<Committed, CommitError> {
        if self.bodies.is_none() {
            self.bodies = Some(BodyStore::new(PackWriter::open(&mut self.project)?));
        }
        // A crash after this commit can lose only exchanges still in memory
        // (none older than the oldest one there) or submitted later.
        let in_batch: HashSet<Ulid> = items.iter().map(|i| i.ex.id()).collect();
        let watermark = self
            .shared
            .oldest_in_memory(&in_batch)
            .map_or_else(slack_ulid, |o| o.min(slack_ulid()));
        #[cfg(test)]
        let hooks = Arc::clone(&self.cfg.hooks);
        let Writer {
            project, bodies, ..
        } = self;
        let Some(bodies) = bodies.as_mut() else {
            return Err(CommitError::Db(StoreError::Closed));
        };
        // (1) BEGIN IMMEDIATE.
        let tx = project
            .writer_mut()
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        // (2) A pending gap: from the earliest lost id to now.
        let marker = match gap {
            Some(from) => Some(insert_gap(
                &tx,
                Some(from),
                "exchanges were dropped: the queue and journal were full or unwritable",
            )?),
            None => None,
        };
        // Packs appended to in this batch, and the end of the last record in each.
        let mut touched: BTreeMap<u32, u64> = BTreeMap::new();
        let mut ids = Vec::with_capacity(items.len());
        for (i, item) in items.iter().enumerate() {
            let id = item.ex.id();
            let exists = tx
                .query_row(
                    "SELECT 1 FROM exchanges WHERE ulid = ?1",
                    [id.to_string()],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            // (3) Bodies and the row; a replayed or repeated id is already stored.
            if !exists {
                #[cfg(test)]
                if hooks
                    .fail_item
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .as_ref()
                    == Some(&id)
                {
                    return Err(CommitError::Item(
                        i,
                        StoreError::Io(std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            "injected bad exchange",
                        )),
                    ));
                }
                insert_exchange(&tx, bodies, &item.ex, &mut touched).map_err(|e| {
                    if per_exchange(&e) {
                        CommitError::Item(i, e)
                    } else {
                        CommitError::Db(e)
                    }
                })?;
            }
            ids.push(id);
        }
        // (4) Advance last_committed_ulid (ULID order; never backwards) and
        // record the watermark gap markers start from after a crash.
        if let Some(max) = ids.iter().max() {
            tx.execute(
                "UPDATE meta SET value = ?1 WHERE key = 'last_committed_ulid' AND value < ?1",
                [max.to_string()],
            )?;
        }
        tx.execute(
            "INSERT INTO meta (key, value) VALUES ('last_commit_watermark', ?1) \
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            [watermark.to_string()],
        )?;
        if !touched.is_empty() {
            // (5) The pack bytes are durable before anything references them.
            bodies.fsync()?;
            for (pack, len) in &touched {
                Project::set_pack_committed_len(&tx, *pack, *len)?;
            }
        }
        #[cfg(test)]
        if hooks
            .fail_commits
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok()
        {
            return Err(CommitError::Db(StoreError::Sqlite(
                rusqlite::Error::SqliteFailure(
                    rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_FULL),
                    Some("injected disk full".into()),
                ),
            )));
        }
        // (6) COMMIT.
        tx.commit()?;
        Ok(Committed { ids, marker })
    }

    fn wal_path(&self) -> PathBuf {
        self.project
            .dir()
            .join(format!("{}-wal", crate::project::DB_FILE))
    }

    fn sync_wal(&self) -> std::io::Result<()> {
        match std::fs::File::open(self.wal_path()).and_then(|f| f.sync_all()) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            r => r,
        }
    }

    fn maybe_checkpoint(&mut self) {
        let wal_len = std::fs::metadata(self.wal_path()).map_or(0, |m| m.len());
        if self.last_checkpoint.elapsed() >= self.cfg.checkpoint_interval
            || wal_len >= self.cfg.checkpoint_bytes
        {
            self.checkpoint("PASSIVE");
            // Make the WAL durable even if readers kept the checkpoint from
            // copying it: this bounds a power loss to one interval.
            if let Err(e) = self.sync_wal() {
                self.shared
                    .events
                    .error(format!("cannot sync the WAL: {e}"));
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

/// Splits `v` into chunks of at most `n`.
fn chunked(mut v: Vec<Item>, n: usize) -> Vec<Vec<Item>> {
    let mut out = Vec::new();
    while v.len() > n {
        let rest = v.split_off(n);
        out.push(v);
        v = rest;
    }
    if !v.is_empty() {
        out.push(v);
    }
    out
}

/// Inserts a `Gap` marker covering `from` (if known) to now and returns it.
fn insert_gap(
    tx: &Transaction<'_>,
    from: Option<Ulid>,
    detail: &str,
) -> Result<Marker, StoreError> {
    let mut marker = Marker::new(MarkerKind::Gap, detail);
    if let Some(from) = from {
        marker = marker.with_from(from);
    }
    let to = marker.id();
    marker = marker.with_to(to);
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
    let host = request_host(ex.request());
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

/// The request's host as stored in `req_host`: the `Host` header value
/// (the message's identity, spec 4.6), ASCII-lowercased with any port kept so
/// the exact filter can use the index. Only when the head has no usable
/// `Host` does it fall back to the connection's target address.
fn request_host(m: &Message) -> Option<String> {
    let from_head = match m.head() {
        Head::Http1Raw { bytes, spans, .. } => spans
            .headers()
            .iter()
            .find(|h| {
                bytes
                    .get(h.name().range())
                    .is_some_and(|n| n.trim_ascii().eq_ignore_ascii_case(b"host"))
            })
            .and_then(|h| bytes.get(h.value().range()))
            .map(|v| String::from_utf8_lossy(v.trim_ascii()).to_ascii_lowercase()),
        Head::Normalized { map } => map
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case("host"))
            .map(|(_, v)| String::from_utf8_lossy(v.trim_ascii()).to_ascii_lowercase()),
        _ => None,
    };
    from_head
        .filter(|h| !h.is_empty())
        .or_else(|| Some(m.connection().target_addr().to_ascii_lowercase()))
        .filter(|h| !h.is_empty())
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
