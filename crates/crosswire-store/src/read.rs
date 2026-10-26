//! The read API: keyset-paginated listing, rehydration, markers and counts.
//!
//! Every statement is parameterised: filter values are bound, never spliced
//! into SQL text. Only fixed fragments are chosen by which filters are set.

use crate::bodies::{BodySide, BodyStore};
use crate::error::StoreError;
use crate::store::Store;
use crosswire_model::{
    Body, Exchange, ExchangeState, Marker, MarkerKind, NamespaceKind, ScopeState,
};
use rusqlite::types::Value as Sql;
use rusqlite::{Connection, OptionalExtension, Row};
use std::ops::RangeInclusive;
use ulid::Ulid;

/// Most readers kept for reuse; more may exist briefly under load.
pub(crate) const READER_POOL_MAX: usize = 4;
/// Largest page [`Store::list`] and [`Store::markers`] return.
pub const PAGE_LIMIT_MAX: usize = 1000;

/// Which exchanges [`Store::list`] returns. Unset fields match everything.
#[derive(Clone, Debug, Default)]
pub struct ExchangeFilter {
    /// Exact request method, for example `GET`.
    pub method: Option<String>,
    /// Exact request host, ASCII case-insensitive. The writer stores the
    /// host lowercased and the bind is lowercased, so this match uses the
    /// host index.
    pub host: Option<String>,
    /// Substring of the request host. ASCII case-insensitive (`lower()` on
    /// both sides). This is a substring match, so it scans rather than
    /// using the host index.
    pub host_contains: Option<String>,
    /// Inclusive range of response status codes.
    pub status: Option<RangeInclusive<u16>>,
    /// The namespace the exchange lives in.
    pub namespace: Option<NamespaceKind>,
    /// The recording state.
    pub state: Option<ExchangeState>,
    /// The scope classification (exact match on the stored `scope_state`).
    pub scope_state: Option<ScopeState>,
    /// Smallest response body length to include, in bytes (inclusive): matches
    /// rows whose `resp_len >= min_resp_len`. The "size over 1 MiB" query is
    /// `Some(1_048_577)`. Served by the `resp_len` index.
    pub min_resp_len: Option<u64>,
    /// A tag the exchange must carry. Matched exactly against the normalized
    /// `exchange_tags` side table, served by its `(tag, ulid)` primary key.
    pub tag: Option<String>,
    /// Case-insensitive (ASCII) substring of the request path. A substring
    /// cannot use a B-tree index, so, like `path_regex`, it is matched in Rust
    /// over the candidates the other predicates leave, reading at most
    /// [`PATH_SCAN_CAP`] of them; [`Page::capped`] reports when that cap cut
    /// the scan short. Combine it with an indexed filter (host, status, tag)
    /// on large projects.
    pub path_contains: Option<String>,
    /// A regular expression the request path must match (unanchored, case
    /// sensitive unless the pattern says `(?i)`). Compiled with the
    /// linear-time `regex` crate, so a hostile pattern cannot hang; an invalid
    /// one is [`StoreError::BadPattern`]. SQLite has no regex, so matching runs
    /// in Rust over candidate rows, reading at most [`PATH_SCAN_CAP`] of them;
    /// [`Page::capped`] reports when that cap cut the scan short.
    pub path_regex: Option<String>,
}

/// Most candidate rows a path (`path_contains`/`path_regex`) listing reads before it stops and
/// reports [`Page::capped`].
pub const PATH_SCAN_CAP: usize = 10_000;

/// Compiled-program size limit for a user regex, in bytes.
const REGEX_SIZE_LIMIT: usize = 1 << 20;

/// One page of results and where the next one starts.
#[derive(Clone, Debug, PartialEq)]
pub struct Page<T> {
    /// The rows, newest first.
    pub items: Vec<T>,
    /// Pass as `after` to get the next page; `None` on the last page.
    pub next_cursor: Option<Ulid>,
    /// `true` when a `path_contains`/`path_regex` scan read its row cap before the page filled,
    /// so matches may exist beyond it; `next_cursor` then resumes after the
    /// last row scanned. Always `false` for indexed and unfiltered listings.
    pub capped: bool,
}

/// A history row, built from the indexed columns without parsing the JSON.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ExchangeRow {
    /// The exchange id.
    pub id: Ulid,
    /// Creation time, unix milliseconds (from the id).
    pub created_at: u64,
    /// Request method, if the head was HTTP/1.
    pub method: Option<String>,
    /// Target host.
    pub host: Option<String>,
    /// Request target (path and query).
    pub path: Option<String>,
    /// Response status, if there was a response.
    pub status: Option<u16>,
    /// Response body length in bytes.
    pub resp_len: Option<u64>,
    /// Recording state.
    pub state: ExchangeState,
    /// Tags.
    pub tags: Vec<String>,
}

/// Totals over the project.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Counts {
    /// All exchanges.
    pub exchanges: u64,
    /// Exchanges in state `committed`.
    pub committed: u64,
    /// Exchanges in state `error`.
    pub error: u64,
    /// Markers.
    pub markers: u64,
    /// Bytes of bodies stored inline.
    pub bytes_inline: u64,
    /// Bytes of records in packs (sum of `blobs.len`).
    pub bytes_packs: u64,
    /// Exchange counts split by namespace.
    pub by_namespace: NamespaceCounts,
}

/// Exchange counts per namespace (a fixed struct rather than a map, so
/// [`Counts`] stays `Copy` and every namespace is always present).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct NamespaceCounts {
    /// Proxy history.
    pub history: u64,
    /// Repeater send history.
    pub repeater: u64,
    /// Intruder attack results.
    pub intruder: u64,
}

fn corrupt(what: &str) -> StoreError {
    StoreError::Corrupt(what.to_owned())
}

fn parse_ulid(s: &str) -> Result<Ulid, StoreError> {
    Ulid::from_string(s).map_err(|_| corrupt("stored id is not a ULID"))
}

fn from_wire<T: serde::de::DeserializeOwned>(s: String) -> Result<T, StoreError> {
    Ok(serde_json::from_value(serde_json::Value::String(s))?)
}

fn to_wire<T: serde::Serialize>(v: &T) -> Result<String, StoreError> {
    match serde_json::to_value(v)? {
        serde_json::Value::String(s) => Ok(s),
        _ => Err(corrupt("enum has no string wire name")),
    }
}

fn unsigned(v: i64) -> u64 {
    u64::try_from(v).unwrap_or(0)
}

const LIST_COLUMNS: &str = "SELECT ulid, created_at, req_method, req_host, req_path, \
     resp_status, resp_len, state, tags FROM exchanges";

/// The list statement and its bound values (values only ever in the params).
fn list_query(
    filter: &ExchangeFilter,
    after: Option<Ulid>,
    limit: usize,
) -> Result<(String, Vec<Sql>), StoreError> {
    let mut sql = String::from(LIST_COLUMNS);
    let mut conds: Vec<&str> = Vec::new();
    let mut params: Vec<Sql> = Vec::new();
    if let Some(a) = after {
        conds.push("ulid < ?");
        params.push(Sql::Text(a.to_string()));
    }
    if let Some(m) = &filter.method {
        conds.push("req_method = ?");
        params.push(Sql::Text(m.clone()));
    }
    if let Some(h) = &filter.host {
        conds.push("req_host = ?");
        params.push(Sql::Text(h.to_ascii_lowercase()));
    }
    if let Some(h) = &filter.host_contains {
        conds.push("instr(lower(req_host), lower(?)) > 0");
        params.push(Sql::Text(h.clone()));
    }
    if let Some(r) = &filter.status {
        conds.push("resp_status BETWEEN ? AND ?");
        params.push(Sql::Integer(i64::from(*r.start())));
        params.push(Sql::Integer(i64::from(*r.end())));
    }
    if let Some(n) = filter.namespace {
        conds.push("namespace = ?");
        params.push(Sql::Text(to_wire(&n)?));
    }
    if let Some(s) = filter.state {
        conds.push("state = ?");
        params.push(Sql::Text(to_wire(&s)?));
    }
    if let Some(ss) = filter.scope_state {
        conds.push("scope_state = ?");
        params.push(Sql::Text(to_wire(&ss)?));
    }
    if let Some(v) = filter.min_resp_len {
        // Pin the covering `resp_len` index (v5). At a million rows the planner
        // otherwise prefers a primary-key scan in ulid order, which wades
        // through ~5 KB rows to find the ~1 % that match (26 ms vs 0.9 ms).
        sql.push_str(" INDEXED BY idx_exchanges_resp_len");
        // `resp_len` is stored as i64 (saturated), so bind a saturated i64.
        conds.push("resp_len >= ?");
        params.push(Sql::Integer(i64::try_from(v).unwrap_or(i64::MAX)));
    }
    if let Some(t) = &filter.tag {
        // The normalized `exchange_tags (tag, ulid)` side table replaces the
        // per-row `json_each` scan: the subquery searches its primary key for
        // `tag = ?`, and the outer `ORDER BY ulid DESC` keyset pages the
        // matching ids straight from the exchanges primary key (EXPLAIN QUERY
        // PLAN: SEARCH exchange_tags USING PRIMARY KEY (tag=?), no SCAN).
        conds.push("ulid IN (SELECT ulid FROM exchange_tags WHERE tag = ?)");
        params.push(Sql::Text(t.clone()));
    }
    // `path_contains` / `path_regex` are not SQL predicates: they are applied
    // by the bounded candidate scan in `Store::list_with_scan_cap`.
    if !conds.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&conds.join(" AND "));
    }
    sql.push_str(" ORDER BY ulid DESC LIMIT ?");
    params.push(Sql::Integer(i64::try_from(limit + 1).unwrap_or(i64::MAX)));
    Ok((sql, params))
}

fn exchange_row(r: &Row<'_>) -> Result<ExchangeRow, StoreError> {
    let id: String = r.get(0)?;
    let tags: String = r.get(8)?;
    let tags = if tags.is_empty() {
        Vec::new()
    } else {
        serde_json::from_str(&tags)?
    };
    Ok(ExchangeRow {
        id: parse_ulid(&id)?,
        created_at: unsigned(r.get(1)?),
        method: r.get(2)?,
        host: r.get(3)?,
        path: r.get(4)?,
        status: r
            .get::<_, Option<i64>>(5)?
            .and_then(|s| u16::try_from(s).ok()),
        resp_len: r.get::<_, Option<i64>>(6)?.map(unsigned),
        state: from_wire(r.get(7)?)?,
        tags,
    })
}

fn paginate<T>(mut items: Vec<T>, limit: usize, id: impl Fn(&T) -> Ulid) -> Page<T> {
    if items.len() > limit {
        items.truncate(limit);
        let next_cursor = items.last().map(id);
        Page {
            items,
            next_cursor,
            capped: false,
        }
    } else {
        Page {
            items,
            next_cursor: None,
            capped: false,
        }
    }
}

impl Store {
    /// Runs `f` on a pooled read-only connection (at most four are kept).
    fn with_reader<T>(
        &self,
        f: impl FnOnce(&Connection) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        let pooled = self
            .readers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pop();
        let conn = match pooled {
            Some(c) => c,
            None => crate::project::open_reader(self.dir())?,
        };
        let out = f(&conn);
        let mut pool = self
            .readers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if pool.len() < READER_POOL_MAX {
            pool.push(conn);
        }
        out
    }

    /// Drops the pooled idle reader connections, so the next read opens a
    /// fresh one with an empty SQLite page cache (cold-query benchmarks).
    #[doc(hidden)]
    pub fn drop_idle_readers(&self) {
        self.readers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }

    /// Lists exchanges newest first (keyset on the ULID).
    ///
    /// `after` is the previous page's `next_cursor`. `limit` is clamped to
    /// `1..=1000`. Rows come from the indexed columns; the stored JSON is
    /// not parsed.
    ///
    /// The keyset is creation time, not commit time: a journal replay (after
    /// a crash or a database failure) can commit ids older than rows a reader
    /// has already paged past, and a poller asking only for newer ids misses
    /// them. A consumer that must see every exchange tails
    /// [`Event::Committed`](crosswire_model::Event::Committed) and uses `list`
    /// for display. A replayed id that was already stored is announced in
    /// `Committed` again, so deduplicate by id.
    pub fn list(
        &self,
        filter: &ExchangeFilter,
        after: Option<Ulid>,
        limit: usize,
    ) -> Result<Page<ExchangeRow>, StoreError> {
        self.list_with_scan_cap(filter, after, limit, PATH_SCAN_CAP)
    }

    /// [`Store::list`] with an explicit path-scan row cap (tests use a small
    /// one; `list` uses [`PATH_SCAN_CAP`]).
    #[doc(hidden)]
    pub fn list_with_scan_cap(
        &self,
        filter: &ExchangeFilter,
        after: Option<Ulid>,
        limit: usize,
        scan_cap: usize,
    ) -> Result<Page<ExchangeRow>, StoreError> {
        let limit = limit.clamp(1, PAGE_LIMIT_MAX);
        let regex = filter
            .path_regex
            .as_deref()
            .map(|p| {
                regex::RegexBuilder::new(p)
                    .size_limit(REGEX_SIZE_LIMIT)
                    .build()
                    .map_err(|e| StoreError::BadPattern(e.to_string()))
            })
            .transpose()?;
        let needle = filter.path_contains.as_deref().map(str::to_ascii_lowercase);
        if regex.is_none() && needle.is_none() {
            let (sql, params) = list_query(filter, after, limit)?;
            let items = self.with_reader(|conn| {
                let mut stmt = conn.prepare(&sql)?;
                let mut rows = stmt.query(rusqlite::params_from_iter(params.iter()))?;
                let mut items = Vec::new();
                while let Some(r) = rows.next()? {
                    items.push(exchange_row(r)?);
                }
                Ok(items)
            })?;
            return Ok(paginate(items, limit, |r| r.id));
        }
        // Path scan: the SQL returns candidates (narrowed by the other
        // predicates, newest first) bounded by the cap; matching is in Rust.
        // Both matchers must accept a row.
        let (sql, params) = list_query(filter, after, scan_cap)?;
        self.with_reader(|conn| {
            let mut stmt = conn.prepare(&sql)?;
            let mut rows = stmt.query(rusqlite::params_from_iter(params.iter()))?;
            let (mut items, mut scanned) = (Vec::new(), 0usize);
            let mut last_scanned = None;
            while let Some(r) = rows.next()? {
                if items.len() > limit {
                    break;
                }
                if scanned >= scan_cap {
                    let mut page = paginate(items, limit, |r: &ExchangeRow| r.id);
                    page.capped = true;
                    page.next_cursor = last_scanned.or(page.next_cursor);
                    return Ok(page);
                }
                scanned += 1;
                let row = exchange_row(r)?;
                last_scanned = Some(row.id);
                let hit = row.path.as_deref().is_some_and(|p| {
                    regex.as_ref().is_none_or(|re| re.is_match(p))
                        && needle
                            .as_ref()
                            .is_none_or(|n| p.to_ascii_lowercase().contains(n.as_str()))
                });
                if hit {
                    items.push(row);
                }
            }
            Ok(paginate(items, limit, |r| r.id))
        })
    }

    /// The query plan of the statement [`Store::list`] would run for
    /// `filter`, one line per plan step (for tests and diagnostics).
    #[doc(hidden)]
    pub fn explain_list(&self, filter: &ExchangeFilter) -> Result<Vec<String>, StoreError> {
        let (sql, params) = list_query(filter, None, 10)?;
        self.with_reader(|conn| {
            let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}"))?;
            let mut rows = stmt.query(rusqlite::params_from_iter(params.iter()))?;
            let mut plan = Vec::new();
            while let Some(r) = rows.next()? {
                plan.push(r.get::<_, String>(3)?);
            }
            Ok(plan)
        })
    }

    /// Loads one exchange with its bodies rehydrated.
    ///
    /// A body stored inline comes back as [`Body::Inline`]. A body in a pack
    /// stays [`Body::Blob`] (hash, length); read its bytes with
    /// [`Store::get_body`]. The stored JSON keeps every body as a `Blob`
    /// reference, so where each lives is read from the `bodies` table.
    pub fn get(&self, id: Ulid) -> Result<Option<Exchange>, StoreError> {
        self.with_reader(|conn| {
            let json: Option<String> = conn
                .query_row(
                    "SELECT json FROM exchanges WHERE ulid = ?1",
                    [id.to_string()],
                    |r| r.get(0),
                )
                .optional()?;
            let Some(json) = json else {
                return Ok(None);
            };
            let mut value: serde_json::Value = serde_json::from_str(&json)?;
            for (side, pointer) in [
                (BodySide::Req, "/request/body"),
                (BodySide::Resp, "/response/body"),
            ] {
                let inline: Option<Option<Vec<u8>>> = conn
                    .query_row(
                        "SELECT inline FROM bodies WHERE ulid = ?1 AND side = ?2",
                        rusqlite::params![id.to_string(), side_name(side)],
                        |r| r.get(0),
                    )
                    .optional()?;
                let Some(Some(bytes)) = inline else { continue };
                let slot = value
                    .pointer_mut(pointer)
                    .ok_or_else(|| corrupt("exchange json lacks a body slot"))?;
                // Keep the capture bookkeeping the stored reference carries.
                let body = match serde_json::from_value::<Body>(slot.take())? {
                    Body::Blob {
                        truncated_at,
                        capture_dropped,
                        relay,
                        ..
                    } => Body::Inline {
                        bytes: bytes::Bytes::from(bytes),
                        truncated_at,
                        capture_dropped,
                        relay,
                    },
                    _ => Body::inline(bytes::Bytes::from(bytes)),
                };
                *slot = serde_json::to_value(body)?;
            }
            Ok(Some(serde_json::from_value(value)?))
        })
    }

    /// The raw bytes of one body, wherever it is stored; `None` if the
    /// exchange has no body on that side. Pack reads are verified.
    pub fn get_body(&self, id: Ulid, side: BodySide) -> Result<Option<Vec<u8>>, StoreError> {
        self.with_reader(|conn| BodyStore::get(conn, self.dir(), id, side))
    }

    /// Lists markers newest first (keyset on the ULID); `limit` is clamped
    /// to `1..=1000`.
    pub fn markers(&self, after: Option<Ulid>, limit: usize) -> Result<Page<Marker>, StoreError> {
        let limit = limit.clamp(1, PAGE_LIMIT_MAX);
        let items = self.with_reader(|conn| {
            let mut stmt = conn.prepare(
                "SELECT ulid, kind, from_ulid, to_ulid, detail FROM markers \
                 WHERE (?1 IS NULL OR ulid < ?1) ORDER BY ulid DESC LIMIT ?2",
            )?;
            let cursor = after.map(|a| a.to_string());
            let take = i64::try_from(limit + 1).unwrap_or(i64::MAX);
            let mut rows = stmt.query(rusqlite::params![cursor, take])?;
            let mut items = Vec::new();
            while let Some(r) = rows.next()? {
                items.push(marker_row(r)?);
            }
            Ok(items)
        })?;
        Ok(paginate(items, limit, Marker::id))
    }

    /// Totals over the project, computed with `COUNT` and `SUM`.
    pub fn counts(&self) -> Result<Counts, StoreError> {
        self.with_reader(|conn| {
            let one = |sql: &str| -> Result<u64, StoreError> {
                Ok(unsigned(conn.query_row(sql, [], |r| r.get(0))?))
            };
            let mut by_namespace = NamespaceCounts::default();
            let mut stmt =
                conn.prepare("SELECT namespace, count(*) FROM exchanges GROUP BY namespace")?;
            let mut rows = stmt.query([])?;
            while let Some(r) = rows.next()? {
                let ns: String = r.get(0)?;
                let count = unsigned(r.get(1)?);
                match ns.as_str() {
                    "history" => by_namespace.history = count,
                    "repeater" => by_namespace.repeater = count,
                    "intruder" => by_namespace.intruder = count,
                    _ => {}
                }
            }
            Ok(Counts {
                exchanges: one("SELECT count(*) FROM exchanges")?,
                committed: one("SELECT count(*) FROM exchanges WHERE state = 'committed'")?,
                error: one("SELECT count(*) FROM exchanges WHERE state = 'error'")?,
                markers: one("SELECT count(*) FROM markers")?,
                bytes_inline: one(
                    "SELECT coalesce(sum(len), 0) FROM bodies WHERE inline IS NOT NULL",
                )?,
                bytes_packs: one("SELECT coalesce(sum(len), 0) FROM blobs")?,
                by_namespace,
            })
        })
    }
}

fn side_name(side: BodySide) -> &'static str {
    match side {
        BodySide::Req => "req",
        BodySide::Resp => "resp",
    }
}

fn marker_row(r: &Row<'_>) -> Result<Marker, StoreError> {
    let id = parse_ulid(&r.get::<_, String>(0)?)?;
    let kind: MarkerKind = from_wire(r.get(1)?)?;
    let mut m = Marker::with_id(id, kind, r.get::<_, String>(4)?);
    if let Some(f) = r.get::<_, Option<String>>(2)? {
        m = m.with_from(parse_ulid(&f)?);
    }
    if let Some(t) = r.get::<_, Option<String>>(3)? {
        m = m.with_to(parse_ulid(&t)?);
    }
    Ok(m)
}
