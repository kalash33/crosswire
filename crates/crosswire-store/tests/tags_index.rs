//! The normalized `exchange_tags` side table that serves the `tag = Y`
//! history query (spec §13) without a full `json_each` scan.
//!
//! What each test pins:
//!
//! * **parity** (correctness) — `Store::list` with a `tag` filter returns
//!   exactly the rows the old `tags <> '' AND EXISTS (json_each(...))` scan
//!   returns, in the same `ulid`-descending order. The scan is run directly on
//!   a reader connection as the independent oracle.
//! * **plan** (performance contract) — `EXPLAIN QUERY PLAN` for the tag query
//!   searches `exchange_tags` by its `(tag, ulid)` primary key and never does a
//!   full `SCAN exchanges`.
//! * **backfill** — a project created before migration 0004 (its
//!   `exchange_tags` table dropped and `user_version` set back to 3) returns the
//!   tagged rows for `tag = Y` after the next open drives the migration.
//! * **dual-write consistency** — `exchange_set_tags` to a new tag makes
//!   `tag = new` match and `tag = old` not match, and clearing an exchange's
//!   tags leaves it no `exchange_tags` rows.

use bytes::Bytes;
use crosswire_h1::{Parser, ParserConfig, Progress, ResponseContext};
use crosswire_model::{Body, ConnectionInfo, Event, Exchange, Message};
use crosswire_store::{EventStream, ExchangeFilter, Project, Store, StoreConfig};
use std::collections::BTreeSet;
use std::path::Path;
use std::time::{Duration, Instant};
use ulid::Ulid;

fn raw_req(host: &str) -> Message {
    let head = format!("GET / HTTP/1.1\r\nHost: {host}\r\n\r\n");
    let mut p = Parser::request(&ParserConfig::default());
    let Progress::Head(raw) = p.feed(head.as_bytes()).unwrap().event else {
        panic!("request head");
    };
    Message::http1_raw(&raw, Body::None, ConnectionInfo::new(host, 443))
}

fn raw_resp() -> Message {
    let head = "HTTP/1.1 200 X\r\n\r\n";
    let mut p = Parser::response(
        &ParserConfig::default(),
        ResponseContext::for_method(b"GET"),
    );
    let Progress::Head(raw) = p.feed(head.as_bytes()).unwrap().event else {
        panic!("response head");
    };
    Message::http1_raw(
        &raw,
        Body::inline(Bytes::from(vec![0u8; 8])),
        ConnectionInfo::new("resp.test", 443),
    )
}

/// An exchange tagged with `tags`.
fn tagged(tags: &[&str]) -> Exchange {
    let mut ex = Exchange::new(raw_req("target.example"));
    ex.set_response(raw_resp());
    for t in tags {
        ex.add_tag(*t);
    }
    ex
}

fn cfg() -> StoreConfig {
    StoreConfig {
        batch_interval: Duration::from_millis(10),
        ..StoreConfig::default()
    }
}

fn create(dir: &Path) -> Store {
    Store::open(Project::create(dir).unwrap(), cfg()).unwrap()
}

fn reopen(dir: &Path) -> Store {
    Store::open(Project::open(dir).unwrap(), cfg()).unwrap()
}

fn wait_committed(ev: &mut EventStream, want: &[Ulid]) {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut got = BTreeSet::new();
    while Instant::now() < deadline && !want.iter().all(|w| got.contains(w)) {
        match ev.try_recv() {
            Some(Event::Committed { ids, .. }) => got.extend(ids),
            Some(_) => {}
            None => std::thread::sleep(Duration::from_millis(2)),
        }
    }
    assert!(want.iter().all(|w| got.contains(w)), "not all committed");
}

fn insert(store: &Store, ev: &mut EventStream, exs: Vec<Exchange>) {
    let ids: Vec<Ulid> = exs.iter().map(Exchange::id).collect();
    let sink = store.sink();
    for ex in exs {
        sink.submit(ex).unwrap();
    }
    wait_committed(ev, &ids);
}

fn tag_filter(tag: &str) -> ExchangeFilter {
    ExchangeFilter {
        tag: Some(tag.into()),
        ..ExchangeFilter::default()
    }
}

/// All ids `list` returns for `filter`, paging to exhaustion, newest first.
fn list_all(store: &Store, filter: &ExchangeFilter) -> Vec<Ulid> {
    let (mut ids, mut after) = (Vec::new(), None);
    loop {
        let page = store
            .list(filter, after, crosswire_store::PAGE_LIMIT_MAX)
            .unwrap();
        ids.extend(page.items.iter().map(|r| r.id));
        match page.next_cursor {
            Some(c) => after = Some(c),
            None => break,
        }
    }
    ids
}

/// The old full-scan oracle: `tag = Y` via `json_each`, newest first, run
/// directly on a reader connection so parity is checked against an independent
/// implementation of the same semantics.
fn scan_oracle(store: &Store, tag: &str) -> Vec<Ulid> {
    let conn = store.reader().unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT ulid FROM exchanges \
             WHERE tags <> '' AND EXISTS (SELECT 1 FROM json_each(tags) WHERE value = ?1) \
             ORDER BY ulid DESC",
        )
        .unwrap();
    stmt.query_map([tag], |r| r.get::<_, String>(0))
        .unwrap()
        .map(|s| Ulid::from_string(&s.unwrap()).unwrap())
        .collect()
}

/// Seeds a tag-diverse project and returns its handles.
fn seed(store: &Store, ev: &mut EventStream) -> Vec<Ulid> {
    let specs: &[&[&str]] = &[
        &["red", "blue"],
        &[],
        &["red"],
        &["blue"],
        &["red", "green"],
        &["green"],
    ];
    let exs: Vec<Exchange> = specs.iter().map(|t| tagged(t)).collect();
    let ids: Vec<Ulid> = exs.iter().map(Exchange::id).collect();
    insert(store, ev, exs);
    ids
}

#[test]
fn tag_query_matches_the_json_each_scan() {
    let tmp = tempfile::tempdir().unwrap();
    let store = create(&tmp.path().join("p"));
    let mut ev = store.subscribe();
    seed(&store, &mut ev);

    for tag in ["red", "blue", "green", "absent"] {
        let want = scan_oracle(&store, tag);
        assert_eq!(
            list_all(&store, &tag_filter(tag)),
            want,
            "tag = {tag}: same rows and order as the json_each scan"
        );
    }
    // The seed really exercises the filter (not all-empty, not all-match).
    assert_eq!(scan_oracle(&store, "red").len(), 3);
    assert_eq!(scan_oracle(&store, "green").len(), 2);
}

#[test]
fn tag_query_uses_the_exchange_tags_index_not_a_scan() {
    let tmp = tempfile::tempdir().unwrap();
    let store = create(&tmp.path().join("p"));
    let mut ev = store.subscribe();
    seed(&store, &mut ev);

    let plan = store.explain_list(&tag_filter("red")).unwrap().join(" | ");
    assert!(
        plan.contains("exchange_tags") && plan.contains("tag=?"),
        "tag query must search exchange_tags by (tag, ulid); plan: {plan}"
    );
    assert!(
        !plan.contains("SCAN exchanges"),
        "tag query must not full-scan exchanges; plan: {plan}"
    );
    println!("tag plan: {plan}");
}

/// Drops `exchange_tags` and sets `user_version` back to 3, so the next open
/// runs the real v3→v4 migration (create + backfill) over existing tagged rows.
fn downgrade_to_v3(db: &Path) {
    let conn = rusqlite::Connection::open(db).unwrap();
    conn.execute_batch("DROP TABLE exchange_tags; PRAGMA user_version = 3;")
        .unwrap();
}

#[test]
fn migration_backfills_tags_for_a_preexisting_project() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("p");
    let db = dir.join("project.db");

    let ids = {
        let store = create(&dir);
        let mut ev = store.subscribe();
        let ids = seed(&store, &mut ev);
        store.close().unwrap();
        ids
    };

    // Pretend it predates the side table, then reopen to drive the migration.
    downgrade_to_v3(&db);
    let store = reopen(&dir);

    let version: u32 = store
        .reader()
        .unwrap()
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, 5, "user_version advanced to current (5)");

    // ids[0] = {red, blue}, ids[2] = {red}, ids[4] = {red, green}, ids[5] = {green}.
    // ULIDs minted in the same millisecond are not monotonic, so order the
    // expectation by ulid (newest first) rather than by insertion order.
    let desc = |mut v: Vec<Ulid>| {
        v.sort_unstable();
        v.reverse();
        v
    };
    assert_eq!(
        list_all(&store, &tag_filter("red")),
        desc(vec![ids[0], ids[2], ids[4]]),
        "backfill covers every pre-0004 tagged row"
    );
    assert_eq!(
        list_all(&store, &tag_filter("green")),
        desc(vec![ids[4], ids[5]]),
    );
}

#[test]
fn set_tags_keeps_the_index_in_lockstep() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("p");
    let store = create(&dir);
    let mut ev = store.subscribe();
    let ex = tagged(&["old"]);
    let id = ex.id();
    insert(&store, &mut ev, vec![ex]);

    assert_eq!(list_all(&store, &tag_filter("old")), vec![id]);

    let control = store.control().unwrap();
    assert!(control.exchange_set_tags(id, &["new".into()]).unwrap());

    assert_eq!(
        list_all(&store, &tag_filter("new")),
        vec![id],
        "tag = new matches after re-tagging"
    );
    assert!(
        list_all(&store, &tag_filter("old")).is_empty(),
        "tag = old no longer matches"
    );

    // Clearing the tags removes every side-table row for the exchange.
    assert!(control.exchange_set_tags(id, &[]).unwrap());
    assert!(list_all(&store, &tag_filter("new")).is_empty());
    let rows: i64 = store
        .reader()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM exchange_tags WHERE ulid = ?1",
            [id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(rows, 0, "a cleared exchange has no exchange_tags rows");
}
