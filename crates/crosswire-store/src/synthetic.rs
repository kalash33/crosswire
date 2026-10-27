//! A deterministic synthetic project for the history benchmarks (spec §13).
//!
//! [`generate`] fills a [`Store`] with `n` exchanges from a seed, with the
//! spread the named history queries need to each match a known, non-empty
//! subset: a fixed host vocabulary, several methods, a known share of 5xx
//! responses, paths with a known `/admin/` segment, the §13 body-size mix, a
//! known in-scope share, and a tag vocabulary with one known tag
//! ([`TAG_Y`]).
//!
//! # Write path
//!
//! The normal write path is the async sink (bounded queue, writer thread, a
//! pack append per body) and is neither fast nor deterministic enough for a
//! million rows. `generate` instead inserts straight through a dedicated
//! write connection in batched transactions, with the same SQL and column
//! shape as the writer's `insert_exchange`: every exchange is a real
//! [`crosswire_model::Exchange`] serialized to the `json` column, state
//! `committed`, with the denormalized `tags` JSON and the matching
//! `exchange_tags` rows. `Store::get` returns every generated row.
//!
//! One deliberate difference: bodies are not stored. The response body in the
//! JSON is a `Blob` reference carrying the sampled length (and a synthetic
//! hash), and no `bodies` row or pack bytes exist, so `Store::get_body` finds
//! nothing. The `resp_len` column, which is all the size query filters on,
//! has the sampled value; writing up to 8 MiB of real bytes for 1 % of a
//! million rows would be tens of gigabytes that no history query reads.

use crate::project::open_control;
use crate::Store;
use crosswire_h1::{Parser, ParserConfig, Progress, ResponseContext};
use crosswire_model::{Body, ConnectionInfo, Exchange, ExchangeState, Message, ScopeState};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use rusqlite::Connection;
use ulid::Ulid;

/// The host a `host = X` query is expected to match (one in eight rows).
pub const HOST_X: &str = "api.example.com";

/// The tag a `tag = Y` query is expected to match (about 5 % of rows).
pub const TAG_Y: &str = "needs-review";

/// The path segment a path-contains query is expected to match (about 8 % of
/// rows); no other generated path contains it.
pub const ADMIN_SEGMENT: &str = "/admin/";

/// The first generated exchange's time (milliseconds since the epoch); each
/// later exchange is one millisecond after the previous, so ULIDs and
/// `created_at` increase with insertion order.
const BASE_MS: u64 = 1_700_000_000_000;

/// Rows per transaction.
const BATCH: usize = 10_000;

const HOSTS: [&str; 8] = [
    HOST_X,
    "www.example.com",
    "cdn.example.com",
    "auth.example.com",
    "shop.example.com",
    "admin.example.com",
    "static.example.com",
    "internal.example.net",
];

/// Methods and their weights (percent).
const METHODS: [(&str, u32); 5] = [
    ("GET", 60),
    ("POST", 25),
    ("PUT", 5),
    ("DELETE", 5),
    ("OPTIONS", 5),
];

/// Statuses and their weights (per mille); 5xx is 5 %.
const STATUSES: [(u16, u32); 10] = [
    (200, 700),
    (304, 80),
    (301, 40),
    (404, 70),
    (403, 30),
    (401, 20),
    (400, 10),
    (500, 25),
    (502, 15),
    (503, 10),
];

const SECTIONS: [&str; 6] = [
    "/api/v1/users",
    "/api/v2/orders",
    "/static/app.js",
    "/login",
    "/search",
    "/products",
];

/// Tags besides [`TAG_Y`], each on about 8 % of rows.
const OTHER_TAGS: [&str; 4] = ["login", "api", "pii", "slow"];

const KIB: u64 = 1024;
const MIB: u64 = 1024 * 1024;

fn pick<T: Copy>(rng: &mut StdRng, table: &[(T, u32)], total: u32) -> T {
    let mut roll = rng.random_range(0..total);
    for (v, w) in table {
        if roll < *w {
            return *v;
        }
        roll -= *w;
    }
    table[0].0
}

/// A response body length from the §13 mix: 60 % under 4 KiB, 30 % 4-64 KiB,
/// 9 % 64 KiB-1 MiB, 1 % over 1 MiB (up to 8 MiB).
fn body_len(rng: &mut StdRng) -> u64 {
    match rng.random_range(0..100) {
        0..=59 => rng.random_range(0..4 * KIB),
        60..=89 => rng.random_range(4 * KIB..64 * KIB),
        90..=98 => rng.random_range(64 * KIB..=MIB),
        _ => rng.random_range(MIB + 1..=8 * MIB),
    }
}

fn request(method: &str, host: &str, path: &str) -> Message {
    let head = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\n\r\n");
    let mut p = Parser::request(&ParserConfig::default());
    let Ok(step) = p.feed(head.as_bytes()) else {
        unreachable!("the generated request head is valid")
    };
    let Progress::Head(raw) = step.event else {
        unreachable!("a complete head yields a head event")
    };
    Message::http1_raw(&raw, Body::None, ConnectionInfo::new(host, 443))
}

fn response(method: &str, host: &str, status: u16, len: u64, hash: [u8; 32]) -> Message {
    let head = format!("HTTP/1.1 {status} X\r\nContent-Length: {len}\r\n\r\n");
    let mut p = Parser::response(
        &ParserConfig::default(),
        ResponseContext::for_method(method.as_bytes()),
    );
    let Ok(step) = p.feed(head.as_bytes()) else {
        unreachable!("the generated response head is valid")
    };
    let Progress::Head(raw) = step.event else {
        unreachable!("a complete head yields a head event")
    };
    let body = if len == 0 {
        Body::None
    } else {
        Body::Blob {
            hash,
            len,
            truncated_at: None,
            capture_dropped: false,
            relay: None,
        }
    };
    Message::http1_raw(&raw, body, ConnectionInfo::new(host, 443))
}

/// One generated exchange and the column values the writer derives from it.
struct Row {
    ex: Exchange,
    method: &'static str,
    host: &'static str,
    path: String,
    status: u16,
    len: u64,
}

fn one(rng: &mut StdRng, i: usize) -> Row {
    let method = pick(rng, &METHODS, 100);
    let host = HOSTS[rng.random_range(0..HOSTS.len())];
    let item: u32 = rng.random_range(0..1_000_000);
    let mut path = if rng.random_range(0..100) < 8 {
        format!("{ADMIN_SEGMENT}panel/{item}")
    } else {
        format!("{}/{item}", SECTIONS[rng.random_range(0..SECTIONS.len())])
    };
    if rng.random_range(0..100) < 20 {
        path.push_str(&format!("?page={}", rng.random_range(1..50)));
    }
    let status = pick(rng, &STATUSES, 1000);
    let len = body_len(rng);
    let mut hash = [0u8; 32];
    rng.fill(&mut hash);
    // One millisecond per row keeps ULIDs strictly increasing; the random
    // part (80 bits) only has to be deterministic.
    let ms = BASE_MS + i as u64;
    let id = Ulid::from_parts(ms, rng.random::<u128>() >> 48);
    let mut ex = Exchange::with_id(id, request(method, host, &path));
    ex.set_response(response(method, host, status, len, hash));
    ex.set_state(ExchangeState::Committed);
    ex.set_scope_state(match rng.random_range(0..100) {
        0..=39 => ScopeState::In,
        40..=89 => ScopeState::Out,
        _ => ScopeState::Empty,
    });
    if rng.random_range(0..100) < 5 {
        ex.add_tag(TAG_Y);
    }
    for t in OTHER_TAGS {
        if rng.random_range(0..100) < 8 {
            ex.add_tag(t);
        }
    }
    Row {
        ex,
        method,
        host,
        path,
        status,
        len,
    }
}

fn scope_name(s: ScopeState) -> &'static str {
    match s {
        ScopeState::In => "in",
        ScopeState::Out => "out",
        _ => "empty",
    }
}

/// Writes `n` synthetic exchanges into `store`, deterministically from `seed`.
///
/// The same `seed` and `n` always produce the same rows (same ULIDs, columns
/// and tags), so per-query match counts are reproducible. See the module
/// docs for the variety and for how this differs from a real recording.
/// Finishes with `ANALYZE`, so SQLite has statistics and the open-ended
/// `resp_len >= ?` size query uses its index instead of scanning.
///
/// # Panics
///
/// If the project cannot be written (this is benchmark and test support, not
/// a recording path).
pub fn generate(store: &Store, n: usize, seed: u64) {
    let mut conn = open_control(store.dir()).expect("open a write connection");
    let mut rng = StdRng::seed_from_u64(seed);
    let mut last = None;
    let mut done = 0;
    while done < n {
        let end = (done + BATCH).min(n);
        let tx = conn.transaction().expect("begin a batch");
        {
            let mut ins = tx
                .prepare_cached(
                    "INSERT INTO exchanges (ulid, namespace, state, scope_state, created_at, \
                     json, req_method, req_host, req_path, resp_status, resp_len, tags, notes) \
                     VALUES (?1, 'history', 'committed', ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, '')",
                )
                .expect("prepare the exchange insert");
            let mut tag_ins = tx
                .prepare_cached("INSERT INTO exchange_tags (ulid, tag) VALUES (?1, ?2)")
                .expect("prepare the tag insert");
            for i in done..end {
                let row = one(&mut rng, i);
                let id = row.ex.id();
                let tags = if row.ex.tags().is_empty() {
                    String::new()
                } else {
                    serde_json::to_string(row.ex.tags()).expect("tags serialize")
                };
                ins.execute(rusqlite::params![
                    id.to_string(),
                    scope_name(row.ex.scope_state()),
                    i64::try_from(id.timestamp_ms()).unwrap_or(i64::MAX),
                    serde_json::to_string(&row.ex).expect("exchange serializes"),
                    row.method,
                    row.host,
                    row.path,
                    row.status,
                    i64::try_from(row.len).unwrap_or(i64::MAX),
                    tags,
                ])
                .expect("insert an exchange");
                for tag in row.ex.tags() {
                    tag_ins
                        .execute(rusqlite::params![id.to_string(), tag])
                        .expect("insert a tag row");
                }
                last = Some(id);
            }
        }
        tx.commit().expect("commit a batch");
        done = end;
    }
    if let Some(max) = last {
        // As the writer does after a batch: the newest committed ULID and the
        // crash-gap watermark.
        advance_meta(&conn, max);
    }
    // Statistics so the planner picks `idx_exchanges_resp_len` for the
    // open-ended size range instead of assuming it matches a quarter of rows.
    conn.execute_batch("ANALYZE;").expect("analyze");
}

fn advance_meta(conn: &Connection, max: Ulid) {
    let max = max.to_string();
    conn.execute(
        "UPDATE meta SET value = ?1 WHERE key = 'last_committed_ulid' AND value < ?1",
        [&max],
    )
    .expect("advance last_committed_ulid");
    conn.execute(
        "INSERT INTO meta (key, value) VALUES ('last_commit_watermark', ?1) \
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        [&max],
    )
    .expect("advance the watermark");
}
