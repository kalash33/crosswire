//! The deterministic synthetic generator: same seed, same data, and the §13
//! variety every named history query needs to match a non-empty subset.
#![cfg(feature = "synthetic")]

use crosswire_model::ScopeState;
use crosswire_store::synthetic::{generate, ADMIN_SEGMENT, HOST_X, TAG_Y};
use crosswire_store::{ExchangeFilter, Project, Store, StoreConfig};

const N: usize = 10_000;

fn fresh(dir: &std::path::Path) -> Store {
    Store::open(Project::create(dir).unwrap(), StoreConfig::default()).unwrap()
}

fn count(store: &Store, f: &ExchangeFilter) -> usize {
    let mut n = 0;
    let mut after = None;
    loop {
        let page = store
            .list(f, after, crosswire_store::PAGE_LIMIT_MAX)
            .unwrap();
        n += page.items.len();
        match page.next_cursor {
            Some(c) => after = Some(c),
            None => return n,
        }
    }
}

fn filters() -> Vec<(&'static str, ExchangeFilter)> {
    vec![
        (
            "host",
            ExchangeFilter {
                host: Some(HOST_X.into()),
                ..Default::default()
            },
        ),
        (
            "5xx post",
            ExchangeFilter {
                status: Some(500..=599),
                method: Some("POST".into()),
                ..Default::default()
            },
        ),
        (
            "size",
            ExchangeFilter {
                min_resp_len: Some(1_048_577),
                ..Default::default()
            },
        ),
        (
            "tag",
            ExchangeFilter {
                tag: Some(TAG_Y.into()),
                ..Default::default()
            },
        ),
        (
            "scope",
            ExchangeFilter {
                scope_state: Some(ScopeState::In),
                ..Default::default()
            },
        ),
        (
            "path",
            ExchangeFilter {
                path_contains: Some(ADMIN_SEGMENT.into()),
                ..Default::default()
            },
        ),
    ]
}

fn counts(store: &Store) -> Vec<usize> {
    filters().iter().map(|(_, f)| count(store, f)).collect()
}

#[test]
fn deterministic_and_every_named_query_is_non_empty() {
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (sa, sb) = (fresh(a.path()), fresh(b.path()));
    generate(&sa, N, 7);
    generate(&sb, N, 7);
    assert_eq!(sa.counts().unwrap().exchanges, N as u64);
    assert_eq!(sb.counts().unwrap().exchanges, N as u64);
    let (ca, cb) = (counts(&sa), counts(&sb));
    assert_eq!(ca, cb, "same seed must give identical match counts");
    for ((name, _), c) in filters().iter().zip(&ca) {
        assert!(*c > 0, "{name} matched nothing");
        assert!(*c < N, "{name} matched everything");
    }
    // A different seed changes the data.
    let c = tempfile::tempdir().unwrap();
    let sc = fresh(c.path());
    generate(&sc, N, 8);
    assert_ne!(counts(&sc), ca);
}

#[test]
fn resp_len_follows_the_body_size_mix() {
    let d = tempfile::tempdir().unwrap();
    let s = fresh(d.path());
    generate(&s, N, 7);
    let conn = s.reader().unwrap();
    let n: i64 = conn
        .query_row(
            "SELECT \
             SUM(resp_len < 4096), \
             SUM(resp_len >= 4096 AND resp_len < 65536), \
             SUM(resp_len >= 65536 AND resp_len <= 1048576), \
             SUM(resp_len > 1048576) FROM exchanges",
            [],
            |r| {
                let v: [i64; 4] = [r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?];
                let want = [0.60, 0.30, 0.09, 0.01];
                for (got, w) in v.iter().zip(want) {
                    let frac = *got as f64 / N as f64;
                    assert!((frac - w).abs() < 0.015, "bucket {frac} vs {w}");
                }
                Ok(v.iter().sum())
            },
        )
        .unwrap();
    assert_eq!(n, N as i64);
}

#[test]
fn rows_are_real_exchanges_with_tag_rows_and_statistics() {
    let d = tempfile::tempdir().unwrap();
    let s = fresh(d.path());
    generate(&s, 500, 3);
    let page = s.list(&ExchangeFilter::default(), None, 5).unwrap();
    for row in &page.items {
        let ex = s.get(row.id).unwrap().expect("json round-trips");
        assert_eq!(ex.id(), row.id);
    }
    let conn = s.reader().unwrap();
    let tag_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM exchange_tags", [], |r| r.get(0))
        .unwrap();
    assert!(tag_rows > 0);
    let stats: i64 = conn
        .query_row("SELECT COUNT(*) FROM sqlite_stat1", [], |r| r.get(0))
        .unwrap();
    assert!(stats > 0, "ANALYZE ran");
    // ULIDs and created_at are monotonic in insertion order.
    let bad: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM (SELECT ulid, created_at, \
             LAG(ulid) OVER (ORDER BY ulid) AS pu, \
             LAG(created_at) OVER (ORDER BY ulid) AS pc FROM exchanges) \
             WHERE pu IS NOT NULL AND (ulid <= pu OR created_at <= pc)",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(bad, 0);
}

#[test]
fn analyze_makes_the_size_query_use_its_index() {
    let d = tempfile::tempdir().unwrap();
    let s = fresh(d.path());
    generate(&s, N, 7);
    let f = ExchangeFilter {
        min_resp_len: Some(1_048_577),
        ..Default::default()
    };
    let plan = s.explain_list(&f).unwrap().join("\n");
    assert!(plan.contains("idx_exchanges_resp_len"), "plan: {plan}");
}
