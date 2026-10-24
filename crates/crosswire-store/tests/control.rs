//! The control path: settings, API tokens, repeater drafts, tags and notes.

use bytes::Bytes;
use crosswire_model::{
    Body, ConnectionInfo, Event, Exchange, Fidelity, FramingRecord, Head, Message, MessageKind,
    ScopeState,
};
use crosswire_store::{EventStream, ExchangeFilter, Project, Store, StoreConfig, TokenScope};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant};
use ulid::Ulid;

fn msg(body: &[u8], host: &str) -> Message {
    Message::new(
        MessageKind::Normalized,
        Fidelity::Normalized,
        Head::Normalized { map: Vec::new() },
        FramingRecord::ContentLength {
            len: body.len() as u64,
        },
        Body::inline(Bytes::copy_from_slice(body)),
        ConnectionInfo::new(host, 443),
    )
}

fn exchange(req: &[u8], host: &str) -> Exchange {
    let mut ex = Exchange::new(msg(req, host));
    ex.set_response(msg(b"ok", host));
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
fn settings_round_trip_and_overwrite() {
    let tmp = tempfile::tempdir().unwrap();
    let store = open(&tmp);
    let control = store.control().unwrap();
    assert_eq!(control.setting_get("theme").unwrap(), None);
    control.setting_put("theme", "dark").unwrap();
    assert_eq!(
        control.setting_get("theme").unwrap().as_deref(),
        Some("dark")
    );
    control.setting_put("theme", "light").unwrap();
    assert_eq!(
        control.setting_get("theme").unwrap().as_deref(),
        Some("light")
    );
    // A second handle sees the committed value.
    let control2 = store.control().unwrap();
    assert_eq!(
        control2.setting_get("theme").unwrap().as_deref(),
        Some("light")
    );
}

#[test]
fn token_create_list_revoke_lookup() {
    let tmp = tempfile::tempdir().unwrap();
    let store = open(&tmp);
    let control = store.control().unwrap();

    let (rec, secret) = control
        .token_create("ci", &[TokenScope::Read, TokenScope::Send])
        .unwrap();
    assert!(secret.starts_with("cwt_"), "secret: {secret}");
    assert_eq!(rec.name, "ci");
    assert_eq!(rec.scopes, vec![TokenScope::Read, TokenScope::Send]);
    assert!(rec.revoked_at.is_none());
    assert!(rec.last_used_at.is_none());

    // The hash is sha256 of the 32 raw secret bytes (the base64url payload).
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    let raw = URL_SAFE_NO_PAD
        .decode(secret.strip_prefix("cwt_").unwrap())
        .unwrap();
    assert_eq!(raw.len(), 32);
    let hash: [u8; 32] = Sha256::digest(&raw).into();

    // Lookup finds the active token and marks it used.
    let found = control.token_lookup(&hash).unwrap().expect("active token");
    assert_eq!(found.id, rec.id);
    assert!(found.last_used_at.is_some());

    // Listing never leaks the hash or the secret.
    let list = control.token_list().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, rec.id);

    // A wrong hash is not found.
    assert!(control.token_lookup(&[0u8; 32]).unwrap().is_none());

    // Revoke: lookup no longer finds it, revoke again is false.
    assert!(control.token_revoke(rec.id).unwrap());
    assert!(control.token_lookup(&hash).unwrap().is_none());
    assert!(!control.token_revoke(rec.id).unwrap());
    assert!(!control.token_revoke(Ulid::new()).unwrap());

    // Still listed, now with revoked_at set.
    let list = control.token_list().unwrap();
    assert_eq!(list.len(), 1);
    assert!(list[0].revoked_at.is_some());
}

#[test]
fn token_last_used_at_updates_at_most_once_per_minute() {
    let tmp = tempfile::tempdir().unwrap();
    let store = open(&tmp);
    let control = store.control().unwrap();
    let (_, secret) = control.token_create("t", &[TokenScope::Admin]).unwrap();
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    let raw = URL_SAFE_NO_PAD
        .decode(secret.strip_prefix("cwt_").unwrap())
        .unwrap();
    let hash: [u8; 32] = Sha256::digest(&raw).into();

    let first = control.token_lookup(&hash).unwrap().unwrap().last_used_at;
    assert!(first.is_some());
    // An immediate second lookup must not move last_used_at (within a minute).
    let second = control.token_lookup(&hash).unwrap().unwrap().last_used_at;
    assert_eq!(first, second);
}

#[test]
fn drafts_round_trip() {
    let tmp = tempfile::tempdir().unwrap();
    let store = open(&tmp);
    let control = store.control().unwrap();
    let tab = Ulid::new();
    assert!(control.draft_get(tab).unwrap().is_none());
    control.draft_put(tab, r#"{"method":"GET"}"#).unwrap();
    assert_eq!(
        control.draft_get(tab).unwrap().as_deref(),
        Some(r#"{"method":"GET"}"#)
    );
    control.draft_put(tab, r#"{"method":"POST"}"#).unwrap();
    assert_eq!(
        control.draft_get(tab).unwrap().as_deref(),
        Some(r#"{"method":"POST"}"#)
    );
    let tab2 = Ulid::new();
    control.draft_put(tab2, "{}").unwrap();
    assert_eq!(control.draft_list().unwrap().len(), 2);
    assert!(control.draft_delete(tab).unwrap());
    assert!(!control.draft_delete(tab).unwrap());
    assert_eq!(control.draft_list().unwrap().len(), 1);
}

#[test]
fn set_tags_and_notes_update_row_and_json() {
    let tmp = tempfile::tempdir().unwrap();
    let store = open(&tmp);
    let mut ev = store.subscribe();
    let ids = insert(&store, &mut ev, vec![exchange(b"a", "a.test")]);
    let id = ids[0];
    let control = store.control().unwrap();

    assert!(control
        .exchange_set_tags(id, &["important".to_string(), "auth".to_string()])
        .unwrap());
    assert!(control.exchange_set_notes(id, "looks juicy").unwrap());

    // The row-level filter finds it by tag.
    let page = store
        .list(
            &ExchangeFilter {
                tag: Some("important".to_string()),
                ..ExchangeFilter::default()
            },
            None,
            10,
        )
        .unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].id, id);
    assert!(page.items[0].tags.contains(&"auth".to_string()));

    // A tag that is not set matches nothing.
    let none = store
        .list(
            &ExchangeFilter {
                tag: Some("nope".to_string()),
                ..ExchangeFilter::default()
            },
            None,
            10,
        )
        .unwrap();
    assert!(none.items.is_empty());

    // get() rebuilds from the json column and must agree.
    let ex = store.get(id).unwrap().unwrap();
    assert_eq!(ex.tags(), &["important".to_string(), "auth".to_string()]);
    assert_eq!(ex.notes(), "looks juicy");

    // A missing exchange returns false, not an error.
    assert!(!control.exchange_set_notes(Ulid::new(), "x").unwrap());
    assert!(!control.exchange_set_tags(Ulid::new(), &[]).unwrap());

    // Clearing tags back to empty keeps the filter and get() consistent.
    assert!(control.exchange_set_tags(id, &[]).unwrap());
    let ex = store.get(id).unwrap().unwrap();
    assert!(ex.tags().is_empty());
    let page = store
        .list(
            &ExchangeFilter {
                tag: Some("important".to_string()),
                ..ExchangeFilter::default()
            },
            None,
            10,
        )
        .unwrap();
    assert!(page.items.is_empty());
}

#[test]
fn scope_state_filter_and_counts_by_namespace() {
    let tmp = tempfile::tempdir().unwrap();
    let store = open(&tmp);
    let mut ev = store.subscribe();
    let mut in_scope = exchange(b"a", "a.test");
    in_scope.set_scope_state(ScopeState::In);
    let mut out_scope = exchange(b"b", "b.test");
    out_scope.set_scope_state(ScopeState::Out);
    insert(&store, &mut ev, vec![in_scope, out_scope]);

    let page = store
        .list(
            &ExchangeFilter {
                scope_state: Some(ScopeState::In),
                ..ExchangeFilter::default()
            },
            None,
            10,
        )
        .unwrap();
    assert_eq!(page.items.len(), 1);

    let counts = store.counts().unwrap();
    assert_eq!(counts.exchanges, 2);
    assert_eq!(counts.by_namespace.history, 2);
    assert_eq!(counts.by_namespace.repeater, 0);
    assert_eq!(counts.by_namespace.intruder, 0);
}

#[test]
fn control_writes_do_not_block_the_writer_thread() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(open(&tmp));
    let mut ev = store.subscribe();

    let sink = store.sink();
    let exs: Vec<Exchange> = (0..1000u32)
        .map(|i| exchange(format!("req-{i}").as_bytes(), "load.test"))
        .collect();
    let ids: Vec<Ulid> = exs.iter().map(Exchange::id).collect();

    let submitter = std::thread::spawn(move || {
        for ex in exs {
            sink.submit(ex).unwrap();
        }
    });

    // 100 control writes concurrent with the writer draining 1,000 exchanges.
    let control = store.control().unwrap();
    for i in 0..100u32 {
        control
            .setting_put(&format!("k{i}"), &format!("v{i}"))
            .expect("control write must not surface SQLITE_BUSY");
    }

    submitter.join().unwrap();
    wait_committed(&mut ev, &ids);

    // Every setting committed.
    for i in 0..100u32 {
        assert_eq!(
            control.setting_get(&format!("k{i}")).unwrap().as_deref(),
            Some(format!("v{i}").as_str())
        );
    }
    assert_eq!(store.counts().unwrap().exchanges, 1000);
}
