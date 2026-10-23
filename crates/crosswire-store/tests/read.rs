//! Read API: keyset pagination, filters, rehydration, markers, counts.

use bytes::Bytes;
use crosswire_model::{
    Body, ConnectionInfo, Event, Exchange, Fidelity, FramingRecord, Head, Message, MessageKind,
};
use crosswire_store::{BodySide, EventStream, ExchangeFilter, Project, Store, StoreConfig};
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant};
use ulid::Ulid;

fn noise(n: usize, seed: u64) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

fn msg(body: &[u8], host: &str) -> Message {
    Message::new(
        MessageKind::Normalized,
        Fidelity::Normalized,
        Head::Normalized { map: Vec::new() },
        FramingRecord::ContentLength {
            len: body.len() as u64,
        },
        Body::Inline {
            bytes: Bytes::copy_from_slice(body),
        },
        ConnectionInfo::new(host, 443),
    )
}

fn exchange(req: &[u8], resp: &[u8], host: &str) -> Exchange {
    let mut ex = Exchange::new(msg(req, host));
    ex.set_response(msg(resp, host));
    ex
}

fn open(tmp: &tempfile::TempDir) -> Store {
    let cfg = StoreConfig {
        batch_interval: Duration::from_millis(10),
        ..StoreConfig::default()
    };
    Store::open(Project::create(&tmp.path().join("p")).unwrap(), cfg).unwrap()
}

fn wait_committed(ev: &mut EventStream, want: &[Ulid]) {
    let deadline = Instant::now() + Duration::from_secs(20);
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

fn insert(store: &Store, ev: &mut EventStream, exs: Vec<Exchange>) -> Vec<Ulid> {
    let ids: Vec<Ulid> = exs.iter().map(Exchange::id).collect();
    let sink = store.sink();
    for ex in exs {
        sink.submit(ex).unwrap();
    }
    wait_committed(ev, &ids);
    ids
}

#[test]
fn keyset_pagination_is_stable_under_inserts() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(open(&tmp));
    let mut ev = store.subscribe();
    let before = insert(
        &store,
        &mut ev,
        (0..25u8).map(|i| exchange(&[i], &[i], "a.test")).collect(),
    );
    let mut ev2 = store.subscribe();
    let writer = {
        let store = Arc::clone(&store);
        std::thread::spawn(move || {
            for _ in 0..5 {
                insert(
                    &store,
                    &mut ev2,
                    (0..10u8).map(|i| exchange(&[i], &[i], "b.test")).collect(),
                );
            }
        })
    };
    let mut seen = Vec::new();
    let mut after = None;
    loop {
        let page = store.list(&ExchangeFilter::default(), after, 10).unwrap();
        assert!(page.items.len() <= 10);
        seen.extend(page.items.iter().map(|r| r.id));
        std::thread::sleep(Duration::from_millis(15));
        match page.next_cursor {
            Some(c) => after = Some(c),
            None => break,
        }
    }
    writer.join().unwrap();
    let unique: BTreeSet<Ulid> = seen.iter().copied().collect();
    assert_eq!(unique.len(), seen.len(), "no duplicates");
    for id in &before {
        assert!(unique.contains(id), "skipped {id}");
    }
    assert!(seen.windows(2).all(|w| w[0] > w[1]), "descending by ulid");
}

#[test]
fn filters_use_indexes() {
    let tmp = tempfile::tempdir().unwrap();
    let store = open(&tmp);
    let by_status = ExchangeFilter {
        status: Some(500..=599),
        ..ExchangeFilter::default()
    };
    let plan = store.explain_list(&by_status).unwrap().join(" | ");
    println!("status plan: {plan}");
    assert!(plan.contains("exchanges_resp_status"), "{plan}");
    let by_ns = ExchangeFilter {
        namespace: Some(crosswire_model::NamespaceKind::History),
        ..ExchangeFilter::default()
    };
    let plan = store.explain_list(&by_ns).unwrap().join(" | ");
    println!("namespace plan: {plan}");
    assert!(plan.contains("exchanges_namespace_created_at"), "{plan}");
    let by_host = ExchangeFilter {
        host_contains: Some("example".into()),
        ..ExchangeFilter::default()
    };
    let plan = store.explain_list(&by_host).unwrap().join(" | ");
    println!("host plan: {plan}");
    // A substring match cannot use the host index: it walks the primary key
    // in ulid order, which is also the keyset order (no sort).
    assert!(!plan.contains("TEMP B-TREE"), "{plan}");
}

#[test]
fn get_rehydrates_bodies() {
    let tmp = tempfile::tempdir().unwrap();
    let store = open(&tmp);
    let mut ev = store.subscribe();
    let big = noise(40_000, 7);
    let small = b"tiny request".to_vec();
    let ex = exchange(&small, &big, "h.test");
    let id = insert(&store, &mut ev, vec![ex])[0];
    let got = store.get(id).unwrap().expect("exists");
    assert_eq!(got.id(), id);
    match got.request().body() {
        Body::Inline { bytes } => assert_eq!(&bytes[..], &small[..]),
        other => panic!("request body: {other:?}"),
    }
    match got.response().expect("response").body() {
        Body::Blob { len, .. } => assert_eq!(*len, 40_000),
        other => panic!("response body: {other:?}"),
    }
    assert_eq!(store.get_body(id, BodySide::Req).unwrap().unwrap(), small);
    assert_eq!(store.get_body(id, BodySide::Resp).unwrap().unwrap(), big);
    assert!(store.get(Ulid::new()).unwrap().is_none());
}

#[test]
fn highly_compressible_body_reads_back() {
    // 2 MiB of zeros compresses far past 100:1; a committed body must read back.
    let tmp = tempfile::tempdir().unwrap();
    let store = open(&tmp);
    let mut ev = store.subscribe();
    let zeros = vec![0u8; 2 * 1024 * 1024];
    let id = insert(&store, &mut ev, vec![exchange(b"req", &zeros, "z.test")])[0];
    assert_eq!(store.get_body(id, BodySide::Resp).unwrap().unwrap(), zeros);
}

#[test]
fn sql_injection_in_filter_is_inert() {
    let tmp = tempfile::tempdir().unwrap();
    let store = open(&tmp);
    let mut ev = store.subscribe();
    insert(
        &store,
        &mut ev,
        (0..5u8).map(|i| exchange(&[i], &[i], "ok.test")).collect(),
    );
    let evil = ExchangeFilter {
        host_contains: Some("x' OR 1=1 --".into()),
        method: Some("GET'; DROP TABLE exchanges; --".into()),
        ..ExchangeFilter::default()
    };
    assert_eq!(store.list(&evil, None, 100).unwrap().items.len(), 0);
    let evil_host_only = ExchangeFilter {
        host_contains: Some("x' OR 1=1 --".into()),
        ..ExchangeFilter::default()
    };
    assert_eq!(
        store.list(&evil_host_only, None, 100).unwrap().items.len(),
        0
    );
    assert_eq!(store.counts().unwrap().exchanges, 5, "table intact");
}

#[test]
fn host_filter_is_case_insensitive_substring_and_limit_is_clamped() {
    let tmp = tempfile::tempdir().unwrap();
    let store = open(&tmp);
    let mut ev = store.subscribe();
    insert(
        &store,
        &mut ev,
        vec![
            exchange(b"a", b"a", "API.Example.test"),
            exchange(b"b", b"b", "other.test"),
        ],
    );
    let f = ExchangeFilter {
        host_contains: Some("EXAMPLE".into()),
        ..ExchangeFilter::default()
    };
    assert_eq!(
        store.list(&f, None, 0).unwrap().items.len(),
        1,
        "limit 0 acts as 1"
    );
    let all = store.list(&ExchangeFilter::default(), None, 1).unwrap();
    assert_eq!(all.items.len(), 1);
    assert!(all.next_cursor.is_some());
    let rest = store
        .list(&ExchangeFilter::default(), all.next_cursor, usize::MAX)
        .unwrap();
    assert_eq!(rest.items.len(), 1);
    assert!(rest.next_cursor.is_none());
}

#[test]
fn counts_and_markers() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("p");
    {
        let store = open(&tmp);
        let mut ev = store.subscribe();
        insert(
            &store,
            &mut ev,
            vec![
                exchange(b"12345", b"abc", "c.test"),
                exchange(b"x", &noise(30_000, 3), "c.test"),
            ],
        );
        // Dropped without close: the next open writes a gap marker.
    }
    let store = Store::open(Project::open(&dir).unwrap(), StoreConfig::default()).unwrap();
    let c = store.counts().unwrap();
    assert_eq!((c.exchanges, c.committed, c.error), (2, 2, 0));
    assert_eq!(c.bytes_inline, 5 + 3 + 1);
    assert!(c.bytes_packs > 0);
    assert_eq!(c.markers, 1);
    let m = store.markers(None, 10).unwrap();
    assert_eq!(m.items.len(), 1);
    assert!(m.next_cursor.is_none());
    assert!(store
        .markers(Some(m.items[0].id()), 10)
        .unwrap()
        .items
        .is_empty());
}

/// A raw HTTP/1 request whose `Host` header is `host`, over a connection to
/// `target` (so the two can differ, as they do for virtual hosts and IPs).
fn raw_request(host: &str, target: &str) -> Exchange {
    let head = format!("GET / HTTP/1.1\r\nHost: {host}\r\n\r\n");
    let mut p = crosswire_h1::Parser::request(&crosswire_h1::ParserConfig::default());
    let step = p.feed(head.as_bytes()).unwrap();
    let crosswire_h1::Progress::Head(raw) = step.event else {
        panic!("no head");
    };
    Exchange::new(Message::http1_raw(
        &raw,
        Body::None,
        ConnectionInfo::new(target, 443),
    ))
}

#[test]
fn exact_host_filter_uses_host_header_is_indexed_and_case_insensitive() {
    let tmp = tempfile::tempdir().unwrap();
    let store = open(&tmp);
    let mut ev = store.subscribe();
    insert(
        &store,
        &mut ev,
        vec![
            raw_request("Example.COM:8443", "203.0.113.7"),
            raw_request("other.test", "203.0.113.7"),
        ],
    );
    let exact = ExchangeFilter {
        host: Some("example.com:8443".into()),
        ..ExchangeFilter::default()
    };
    let plan = store.explain_list(&exact).unwrap().join(" | ");
    println!("exact host plan: {plan}");
    assert!(plan.contains("exchanges_req_host"), "{plan}");
    let page = store.list(&exact, None, 10).unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].host.as_deref(), Some("example.com:8443"));
    let upper = ExchangeFilter {
        host: Some("EXAMPLE.com:8443".into()),
        ..ExchangeFilter::default()
    };
    assert_eq!(store.list(&upper, None, 10).unwrap().items.len(), 1);
    let sub = ExchangeFilter {
        host_contains: Some("AMPLE".into()),
        ..ExchangeFilter::default()
    };
    assert_eq!(store.list(&sub, None, 10).unwrap().items.len(), 1);
    let by_ip = ExchangeFilter {
        host: Some("203.0.113.7".into()),
        ..ExchangeFilter::default()
    };
    assert_eq!(
        store.list(&by_ip, None, 10).unwrap().items.len(),
        0,
        "the connection IP is not the host"
    );
}

#[test]
fn host_falls_back_to_target_addr_without_a_host_header() {
    let tmp = tempfile::tempdir().unwrap();
    let store = open(&tmp);
    let mut ev = store.subscribe();
    let on = |host: &str, port: u16, tls: bool| {
        let mut conn = ConnectionInfo::new(host, port);
        if tls {
            conn = conn.with_tls("TLSv1_3", "TLS_AES_128_GCM_SHA256");
        }
        let m = Message::new(
            MessageKind::Normalized,
            Fidelity::Normalized,
            Head::Normalized { map: Vec::new() },
            FramingRecord::None,
            Body::None,
            conn,
        );
        Exchange::new(m)
    };
    insert(
        &store,
        &mut ev,
        vec![
            exchange(b"a", b"a", "Fallback.TEST"),
            on("203.0.113.7", 8443, true),
            on("203.0.113.8", 80, false),
            on("203.0.113.9", 443, true),
            on("203.0.113.10", 443, false),
        ],
    );
    let count = |h: &str| {
        let f = ExchangeFilter {
            host: Some(h.into()),
            ..ExchangeFilter::default()
        };
        store.list(&f, None, 10).unwrap().items.len()
    };
    // `exchange` has no TLS and port 443: not the plaintext default.
    assert_eq!(count("fallback.test:443"), 1);
    assert_eq!(count("203.0.113.7:8443"), 1, "non-default port is kept");
    assert_eq!(count("203.0.113.8"), 1, "plaintext 80 is the default");
    assert_eq!(count("203.0.113.9"), 1, "TLS 443 is the default");
    assert_eq!(count("203.0.113.10:443"), 1, "plaintext 443 is not");
}
