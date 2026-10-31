//! Structured secondary indexes (§13 history queries), the `min_resp_len`
//! size filter.
//!
//! What each test pins:
//!
//! * **parity** (correctness) — `Store::list` returns exactly the rows a
//!   filter's semantics demand, in the same `ulid`-descending order, matching
//!   an expectation computed independently in Rust from the known seed. For the
//!   host query it also cross-checks the substring (`host_contains`) path,
//!   which provably scans the primary key in ulid order rather than using the
//!   host index, so identical rows prove the index did not change the result.
//! * **plan** (performance contract, §3.5) — `EXPLAIN QUERY PLAN` searches the
//!   expected index and never does a full `SCAN exchanges`. The three
//!   equality-led queries (host / method+status / scope) use their index on a
//!   fresh project. The open-ended `resp_len >= ?` range only uses its index
//!   once the planner has statistics for the (skewed) size distribution, which
//!   an `ANALYZE` provides — so its plan is asserted on an analysed, populated
//!   project (the §13 scenario), in `analysed_*`.
//! * **keyset paging** — a filtered result paged to exhaustion via
//!   `next_cursor` yields every matching id exactly once, descending.

use bytes::Bytes;
use crosswire_h1::{Parser, ParserConfig, Progress, ResponseContext};
use crosswire_model::{Body, ConnectionInfo, Event, Exchange, Message, ScopeState};
use crosswire_store::{EventStream, ExchangeFilter, Project, Store, StoreConfig};
use std::collections::BTreeSet;
use std::path::Path;
use std::time::{Duration, Instant};
use ulid::Ulid;

/// "size > 1 MiB" as a `min_resp_len`: `resp_len >= 1_048_577` is exactly
/// `resp_len > 1_048_576`, so the 1 MiB boundary row is excluded.
const OVER_1_MIB: u64 = 1_048_577;

/// What the denormalized columns hold for one seeded exchange, so each test can
/// compute the expected filtered result without going through SQLite.
#[derive(Clone)]
struct Attrs {
    id: Ulid,
    host: String,
    method: String,
    status: u16,
    resp_len: u64,
    scope: ScopeState,
}

/// A raw HTTP/1 request with method `method` and a `Host` header of `host`.
fn raw_req(method: &str, host: &str) -> Message {
    let head = format!("{method} / HTTP/1.1\r\nHost: {host}\r\n\r\n");
    let mut p = Parser::request(&ParserConfig::default());
    let Progress::Head(raw) = p.feed(head.as_bytes()).unwrap().event else {
        panic!("request head");
    };
    Message::http1_raw(&raw, Body::None, ConnectionInfo::new(host, 443))
}

/// A raw HTTP/1 response with status `status` and a `body_len`-byte body (so
/// the stored `resp_len` is exactly `body_len`).
fn raw_resp(status: u16, body_len: usize) -> Message {
    let head = format!("HTTP/1.1 {status} X\r\n\r\n");
    let mut p = Parser::response(
        &ParserConfig::default(),
        ResponseContext::for_method(b"GET"),
    );
    let Progress::Head(raw) = p.feed(head.as_bytes()).unwrap().event else {
        panic!("response head");
    };
    Message::http1_raw(
        &raw,
        Body::inline(Bytes::from(vec![0u8; body_len])),
        ConnectionInfo::new("resp.test", 443),
    )
}

fn build(host: &str, method: &str, status: u16, body_len: usize, scope: ScopeState) -> Exchange {
    let mut ex = Exchange::new(raw_req(method, host));
    ex.set_response(raw_resp(status, body_len));
    ex.set_scope_state(scope);
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

/// Seeds a diverse set covering every structured query and returns the known
/// per-row attributes.
fn seed(store: &Store, ev: &mut EventStream) -> Vec<Attrs> {
    use ScopeState::{Empty, HostMismatch, In, Out};
    // (host, method, status, body_len, scope)
    let specs: &[(&str, &str, u16, usize, ScopeState)] = &[
        ("target.example", "GET", 200, 10, Empty),
        ("target.example", "POST", 500, 1_048_577, In),
        ("target.example", "GET", 404, 1_048_576, Out),
        ("other.example", "POST", 503, 1_100_000, In),
        ("other.example", "POST", 200, 50, HostMismatch),
        ("other.example", "GET", 500, 500, Out),
        ("api.example", "POST", 500, 1_150_000, In),
        ("api.example", "GET", 200, 1_000, Empty),
        ("c.example", "DELETE", 500, 1_200_000, HostMismatch),
        ("c.example", "GET", 301, 1_048_576, Empty),
    ];
    let (mut exs, mut attrs) = (Vec::new(), Vec::new());
    for &(host, method, status, body_len, scope) in specs {
        let ex = build(host, method, status, body_len, scope);
        attrs.push(Attrs {
            id: ex.id(),
            host: host.to_owned(),
            method: method.to_owned(),
            status,
            resp_len: body_len as u64,
            scope,
        });
        exs.push(ex);
    }
    insert(store, ev, exs);
    attrs
}

/// Seeds `n` rows with the §13-style skewed size mix: every 50th response is
/// just over 1 MiB, the rest tiny. Rare large rows are what make the planner
/// pick the `resp_len` index over a ulid-ordered scan once analysed.
fn seed_skewed(store: &Store, ev: &mut EventStream, n: usize) -> Vec<Attrs> {
    let (mut exs, mut attrs) = (Vec::new(), Vec::new());
    for i in 0..n {
        let body = if i % 50 == 49 { 1_200_000 } else { 64 + i % 7 };
        let host = format!("h{}.example", i % 5);
        let ex = build(&host, "GET", 200, body, ScopeState::Empty);
        attrs.push(Attrs {
            id: ex.id(),
            host,
            method: "GET".to_owned(),
            status: 200,
            resp_len: body as u64,
            scope: ScopeState::Empty,
        });
        exs.push(ex);
    }
    insert(store, ev, exs);
    attrs
}

/// Every matching id, newest first — the order `list` must reproduce.
fn expected(attrs: &[Attrs], pred: impl Fn(&Attrs) -> bool) -> Vec<Ulid> {
    let mut ids: Vec<Ulid> = attrs.iter().filter(|a| pred(a)).map(|a| a.id).collect();
    ids.sort_unstable();
    ids.reverse();
    ids
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

/// The plan searches `index` and never does a full table scan.
fn assert_uses(store: &Store, filter: &ExchangeFilter, index: &str) {
    let plan = store.explain_list(filter).unwrap().join(" | ");
    assert!(
        plan.contains(&format!("USING INDEX {index}"))
            || plan.contains(&format!("USING COVERING INDEX {index}")),
        "expected {index}; plan: {plan}"
    );
    assert!(!plan.contains("SCAN exchanges"), "full scan; plan: {plan}");
    println!("{index} plan: {plan}");
}

fn size_filter() -> ExchangeFilter {
    ExchangeFilter {
        min_resp_len: Some(OVER_1_MIB),
        ..ExchangeFilter::default()
    }
}

#[test]
fn host_query_is_indexed_and_matches_the_scan() {
    let tmp = tempfile::tempdir().unwrap();
    let store = create(&tmp.path().join("p"));
    let mut ev = store.subscribe();
    let attrs = seed(&store, &mut ev);

    let filter = ExchangeFilter {
        host: Some("target.example".into()),
        ..ExchangeFilter::default()
    };
    let want = expected(&attrs, |a| a.host == "target.example");
    assert_eq!(list_all(&store, &filter), want, "host rows/order");

    // The substring path cannot use the host index (read.rs proves it walks the
    // primary key, i.e. ulid order); identical rows prove index parity vs scan.
    let scan = ExchangeFilter {
        host_contains: Some("target.example".into()),
        ..ExchangeFilter::default()
    };
    assert_eq!(list_all(&store, &scan), want, "index result equals scan");

    assert_uses(&store, &filter, "idx_exchanges_host");
}

#[test]
fn method_status_query_is_indexed() {
    let tmp = tempfile::tempdir().unwrap();
    let store = create(&tmp.path().join("p"));
    let mut ev = store.subscribe();
    let attrs = seed(&store, &mut ev);

    let filter = ExchangeFilter {
        method: Some("POST".into()),
        status: Some(500..=u16::MAX),
        ..ExchangeFilter::default()
    };
    let want = expected(&attrs, |a| a.method == "POST" && a.status >= 500);
    assert!(!want.is_empty());
    assert_eq!(list_all(&store, &filter), want, "status>=500 & POST");
    assert_uses(&store, &filter, "idx_exchanges_method_status");
    // The v5 shape `(req_method, ulid, resp_status)` yields ulid order, so the
    // status range must not need a temp sort (the 12 s regression at 1M rows).
    let plan = store.explain_list(&filter).unwrap().join("\n");
    assert!(!plan.contains("TEMP B-TREE"), "no temp sort: {plan}");
}

#[test]
fn scope_query_is_indexed() {
    let tmp = tempfile::tempdir().unwrap();
    let store = create(&tmp.path().join("p"));
    let mut ev = store.subscribe();
    let attrs = seed(&store, &mut ev);

    let filter = ExchangeFilter {
        scope_state: Some(ScopeState::In),
        ..ExchangeFilter::default()
    };
    let want = expected(&attrs, |a| a.scope == ScopeState::In);
    assert!(!want.is_empty());
    assert_eq!(list_all(&store, &filter), want, "scope = in");
    assert_uses(&store, &filter, "idx_exchanges_scope");
}

#[test]
fn size_filter_excludes_the_1_mib_boundary_and_matches_expectation() {
    let tmp = tempfile::tempdir().unwrap();
    let store = create(&tmp.path().join("p"));
    let mut ev = store.subscribe();
    let attrs = seed(&store, &mut ev);

    // Correctness of the predicate and the ulid ordering, independent of the
    // access path: the 1_048_576-byte rows are excluded, 1_048_577 included.
    let want = expected(&attrs, |a| a.resp_len >= OVER_1_MIB);
    assert!(!want.is_empty());
    assert!(
        attrs.iter().any(|a| a.resp_len == 1_048_576),
        "seed must include a boundary row to exclude"
    );
    assert_eq!(
        list_all(&store, &size_filter()),
        want,
        "resp_len >= 1 MiB + 1"
    );
}

#[test]
fn keyset_paging_a_filtered_host_is_exhaustive() {
    let tmp = tempfile::tempdir().unwrap();
    let store = create(&tmp.path().join("p"));
    let mut ev = store.subscribe();
    // More rows than a page, all one host, interleaved with other hosts.
    let (mut exs, mut want) = (Vec::new(), BTreeSet::new());
    for i in 0..60u32 {
        let ex = build("page.test", "GET", 200, (i % 7) as usize, ScopeState::Empty);
        want.insert(ex.id());
        exs.push(ex);
        exs.push(build("noise.test", "GET", 200, 1, ScopeState::Empty));
    }
    insert(&store, &mut ev, exs);

    let filter = ExchangeFilter {
        host: Some("page.test".into()),
        ..ExchangeFilter::default()
    };
    let (mut seen, mut after) = (Vec::new(), None);
    loop {
        let page = store.list(&filter, after, 7).unwrap();
        assert!(page.items.len() <= 7);
        for r in &page.items {
            assert_eq!(r.host.as_deref(), Some("page.test"));
            seen.push(r.id);
        }
        match page.next_cursor {
            Some(c) => after = Some(c),
            None => break,
        }
    }
    let unique: BTreeSet<Ulid> = seen.iter().copied().collect();
    assert_eq!(unique.len(), seen.len(), "no id twice");
    assert_eq!(unique, want, "every matching id exactly once");
    assert!(seen.windows(2).all(|w| w[0] > w[1]), "descending by ulid");
    assert_uses(&store, &filter, "idx_exchanges_host");
}

/// Gathers planner statistics on a closed project, as the retired v2→v3
/// migration did, so the open-ended `resp_len >= ?` range can pick its index.
fn analyse(db: &Path) {
    let conn = rusqlite::Connection::open(db).unwrap();
    conn.execute_batch("ANALYZE;").unwrap();
}

#[test]
fn analysed_populated_project_serves_size_query_from_its_index() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("p");
    let db = dir.join("project.db");

    // Seed a populated project with rare large responses, then close cleanly.
    let (attrs, before) = {
        let store = create(&dir);
        let mut ev = store.subscribe();
        let attrs = seed_skewed(&store, &mut ev, 250);
        let before = list_all(&store, &size_filter());
        store.close().unwrap();
        (attrs, before)
    };
    assert_eq!(before.len(), 5, "every 50th of 250 rows is over 1 MiB");

    analyse(&db);
    let store = reopen(&dir);

    let conn = store.reader().unwrap();
    let version: u32 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, 6, "project stays at the current format (6)");
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='index' AND name LIKE 'idx_exchanges_%' ORDER BY name")
        .unwrap();
    let indexes: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        indexes,
        vec![
            "idx_exchanges_host",
            "idx_exchanges_method_status",
            "idx_exchanges_resp_len",
            "idx_exchanges_scope",
        ],
        "all four indexes present"
    );

    let want = expected(&attrs, |a| a.resp_len >= OVER_1_MIB);
    assert_eq!(list_all(&store, &size_filter()), want, "size parity");
    assert_eq!(list_all(&store, &size_filter()), before);

    // With statistics the selective range uses its index rather than scanning
    // the whole table (the §13 performance contract).
    assert_uses(&store, &size_filter(), "idx_exchanges_resp_len");
}
