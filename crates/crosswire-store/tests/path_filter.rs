//! The bounded path filters (§13 queries 4 and 8): `path_contains` is a SQL
//! case-insensitive substring over `req_path`; `path_regex` compiles with the
//! linear-time `regex` crate and filters candidate rows in Rust, reading at most
//! a row cap. An invalid pattern is a typed error, never a panic.

use bytes::Bytes;
use crosswire_h1::{Parser, ParserConfig, Progress, ResponseContext};
use crosswire_model::{Body, ConnectionInfo, Event, Exchange, Message};
use crosswire_store::{EventStream, ExchangeFilter, Project, Store, StoreConfig, StoreError};
use std::collections::BTreeSet;
use std::time::{Duration, Instant};
use ulid::Ulid;

fn raw_req(host: &str, path: &str) -> Message {
    let head = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\n\r\n");
    let mut p = Parser::request(&ParserConfig::default());
    let Progress::Head(raw) = p.feed(head.as_bytes()).unwrap().event else {
        panic!("request head");
    };
    Message::http1_raw(&raw, Body::None, ConnectionInfo::new(host, 443))
}

fn raw_resp() -> Message {
    let mut p = Parser::response(
        &ParserConfig::default(),
        ResponseContext::for_method(b"GET"),
    );
    let Progress::Head(raw) = p.feed(b"HTTP/1.1 200 X\r\n\r\n").unwrap().event else {
        panic!("response head");
    };
    Message::http1_raw(
        &raw,
        Body::inline(Bytes::from(vec![0u8; 4])),
        ConnectionInfo::new("resp.test", 443),
    )
}

fn exchange(host: &str, path: &str) -> Exchange {
    let mut ex = Exchange::new(raw_req(host, path));
    ex.set_response(raw_resp());
    ex
}

fn open(dir: &std::path::Path) -> (Store, EventStream) {
    let cfg = StoreConfig {
        batch_interval: Duration::from_millis(10),
        ..StoreConfig::default()
    };
    let store = Store::open(Project::create(dir).unwrap(), cfg).unwrap();
    let ev = store.subscribe();
    (store, ev)
}

/// Inserts `(host, path)` rows, waits for commit, returns ids in input order.
fn seed(store: &Store, ev: &mut EventStream, rows: &[(&str, String)]) -> Vec<Ulid> {
    let exs: Vec<Exchange> = rows.iter().map(|(h, p)| exchange(h, p)).collect();
    let ids: Vec<Ulid> = exs.iter().map(Exchange::id).collect();
    let sink = store.sink();
    for ex in exs {
        sink.submit(ex).unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut got = BTreeSet::new();
    while Instant::now() < deadline && !ids.iter().all(|i| got.contains(i)) {
        match ev.try_recv() {
            Some(Event::Committed { ids, .. }) => got.extend(ids),
            Some(_) => {}
            None => std::thread::sleep(Duration::from_millis(2)),
        }
    }
    assert!(ids.iter().all(|i| got.contains(i)), "not all committed");
    ids
}

fn ids_of(store: &Store, f: &ExchangeFilter) -> Vec<Ulid> {
    store
        .list(f, None, 1000)
        .unwrap()
        .items
        .iter()
        .map(|r| r.id)
        .collect()
}

fn newest_first(mut v: Vec<Ulid>) -> Vec<Ulid> {
    v.sort();
    v.reverse();
    v
}

fn base_rows() -> Vec<(&'static str, String)> {
    vec![
        ("a.example", "/api/Users/1".into()),
        ("a.example", "/static/app.js".into()),
        ("b.example", "/API/orders".into()),
        ("b.example", "/api/users/2".into()),
        ("a.example", "/login".into()),
    ]
}

#[test]
fn path_contains_is_case_insensitive_substring() {
    let dir = tempfile::tempdir().unwrap();
    let (store, mut ev) = open(dir.path());
    let ids = seed(&store, &mut ev, &base_rows());
    let f = ExchangeFilter {
        path_contains: Some("api/user".into()),
        ..ExchangeFilter::default()
    };
    assert_eq!(
        ids_of(&store, &f),
        newest_first(vec![ids[0], ids[3]]),
        "only the two /api/users paths"
    );
}

#[test]
fn path_regex_matches_compiled_pattern() {
    let dir = tempfile::tempdir().unwrap();
    let (store, mut ev) = open(dir.path());
    let ids = seed(&store, &mut ev, &base_rows());
    let f = ExchangeFilter {
        path_regex: Some(r"^/api/users/\d+$".into()),
        ..ExchangeFilter::default()
    };
    // Case-sensitive: `/api/Users/1` does not match, `/api/users/2` does.
    assert_eq!(ids_of(&store, &f), vec![ids[3]]);
    let js = ExchangeFilter {
        path_regex: Some(r"\.js$".into()),
        ..ExchangeFilter::default()
    };
    let page = store.list(&js, None, 10).unwrap();
    assert_eq!(
        page.items.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![ids[1]]
    );
    assert!(!page.capped);
}

#[test]
fn invalid_regex_is_a_typed_error_not_a_panic() {
    let dir = tempfile::tempdir().unwrap();
    let (store, _ev) = open(dir.path());
    let f = ExchangeFilter {
        path_regex: Some("(".into()),
        ..ExchangeFilter::default()
    };
    assert!(matches!(
        store.list(&f, None, 10),
        Err(StoreError::BadPattern(_))
    ));
}

#[test]
fn path_filter_intersects_with_host() {
    let dir = tempfile::tempdir().unwrap();
    let (store, mut ev) = open(dir.path());
    let ids = seed(&store, &mut ev, &base_rows());
    let contains = ExchangeFilter {
        host: Some("b.example".into()),
        path_contains: Some("users".into()),
        ..ExchangeFilter::default()
    };
    assert_eq!(ids_of(&store, &contains), vec![ids[3]]);
    let regex = ExchangeFilter {
        host: Some("a.example".into()),
        path_regex: Some("^/(login|static)".into()),
        ..ExchangeFilter::default()
    };
    assert_eq!(ids_of(&store, &regex), newest_first(vec![ids[1], ids[4]]));
    // The host index still narrows the candidates.
    let plan = store.explain_list(&contains).unwrap().join("\n");
    assert!(
        plan.contains("req_host") || plan.contains("USING INDEX"),
        "{plan}"
    );
    assert!(!plan.contains("SCAN exchanges"), "{plan}");
}

#[test]
fn regex_scan_is_capped_and_flagged() {
    let dir = tempfile::tempdir().unwrap();
    let (store, mut ev) = open(dir.path());
    let rows: Vec<(&str, String)> = (0..30).map(|i| ("h.example", format!("/p/{i}"))).collect();
    let ids = seed(&store, &mut ev, &rows);
    // Nothing matches, 30 candidates, cap 10: scan stops at the cap.
    let none = ExchangeFilter {
        path_regex: Some("^/never$".into()),
        ..ExchangeFilter::default()
    };
    let page = store.list_with_scan_cap(&none, None, 100, 10).unwrap();
    assert!(page.items.is_empty());
    assert!(page.capped);
    // The cursor resumes after the last scanned row (the 10th newest).
    let newest = newest_first(ids.clone());
    assert_eq!(page.next_cursor, Some(newest[9]));
    // Cap larger than the table: complete, not capped.
    let page = store.list_with_scan_cap(&none, None, 100, 30).unwrap();
    assert!(!page.capped);
    // A page that fills before the cap is not flagged.
    let all = ExchangeFilter {
        path_regex: Some("^/p/".into()),
        ..ExchangeFilter::default()
    };
    let page = store.list_with_scan_cap(&all, None, 5, 10).unwrap();
    assert_eq!(page.items.len(), 5);
    assert!(!page.capped);
    assert!(page.next_cursor.is_some());
}

#[test]
fn plain_listing_is_never_capped() {
    let dir = tempfile::tempdir().unwrap();
    let (store, mut ev) = open(dir.path());
    seed(&store, &mut ev, &base_rows());
    let page = store.list(&ExchangeFilter::default(), None, 10).unwrap();
    assert!(!page.capped);
}
