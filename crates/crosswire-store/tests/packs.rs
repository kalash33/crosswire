//! Body storage: inline/pack split, dedupe, hash-verified reads, startup sweep.

use crosswire_store::{sweep, BodySide, BodyStore, PackWriter, Project, StoreError};
use sha2::{Digest, Sha256};
use std::fs::OpenOptions;
use std::io::Write;
use ulid::Ulid;

/// Deterministic, poorly compressible bytes.
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

fn setup() -> (tempfile::TempDir, Project) {
    let tmp = tempfile::tempdir().unwrap();
    let p = Project::create(&tmp.path().join("p")).unwrap();
    (tmp, p)
}

fn add_exchange(tx: &rusqlite::Transaction, ulid: Ulid) {
    tx.execute(
        "INSERT INTO exchanges (ulid, namespace, state, scope_state, created_at, json) \
         VALUES (?1, 'default', 'complete', 'in', 0, '{}')",
        [ulid.to_string()],
    )
    .unwrap();
}

fn put_one(p: &mut Project, store: &mut BodyStore, raw: &[u8]) -> Ulid {
    let id = Ulid::new();
    let tx = p.writer_mut().transaction().unwrap();
    add_exchange(&tx, id);
    store.put(&tx, id, BodySide::Resp, raw).unwrap();
    tx.commit().unwrap();
    id
}

#[test]
fn inline_boundary() {
    let (_t, mut p) = setup();
    let mut store = BodyStore::new(PackWriter::open(&mut p).unwrap());
    let small = noise(16 * 1024, 1);
    let big = noise(16 * 1024 + 1, 2);
    let a = put_one(&mut p, &mut store, &small);
    let b = put_one(&mut p, &mut store, &big);
    let c = p.writer_mut();
    let inline_a: bool = c
        .query_row(
            "SELECT inline IS NOT NULL FROM bodies WHERE ulid = ?1",
            [a.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    let inline_b: bool = c
        .query_row(
            "SELECT inline IS NOT NULL FROM bodies WHERE ulid = ?1",
            [b.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert!(inline_a && !inline_b);
    assert_eq!(
        BodyStore::get(&p.reader().unwrap(), p.dir(), a, BodySide::Resp)
            .unwrap()
            .unwrap(),
        small
    );
    assert_eq!(
        BodyStore::get(&p.reader().unwrap(), p.dir(), b, BodySide::Resp)
            .unwrap()
            .unwrap(),
        big
    );
    assert!(
        BodyStore::get(&p.reader().unwrap(), p.dir(), a, BodySide::Req)
            .unwrap()
            .is_none()
    );
}

#[test]
fn dedupe_by_hash() {
    let (_t, mut p) = setup();
    let mut store = BodyStore::new(PackWriter::open(&mut p).unwrap());
    let body = noise(20 * 1024, 3);
    let a = put_one(&mut p, &mut store, &body);
    let b = put_one(&mut p, &mut store, &body);
    let c = p.writer_mut();
    let (n, rc): (i64, i64) = c
        .query_row("SELECT count(*), sum(refcount) FROM blobs", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!((n, rc), (1, 2));
    for id in [a, b] {
        assert_eq!(
            BodyStore::get(&p.reader().unwrap(), p.dir(), id, BodySide::Resp)
                .unwrap()
                .unwrap(),
            body
        );
    }
}

#[test]
fn compressible_small_body_roundtrips() {
    let (_t, mut p) = setup();
    let mut store = BodyStore::new(PackWriter::open(&mut p).unwrap());
    let body = vec![0u8; 100 * 1024];
    let a = put_one(&mut p, &mut store, &body);
    let got = BodyStore::get(&p.reader().unwrap(), p.dir(), a, BodySide::Resp)
        .unwrap()
        .unwrap();
    assert_eq!(got, body);
}

fn corrupt_msg(r: Result<Option<Vec<u8>>, StoreError>) -> String {
    match r {
        Err(StoreError::Corrupt(m)) => m,
        other => panic!("expected Corrupt, got {other:?}"),
    }
}

#[test]
fn hash_mismatch_detected_on_read() {
    let (_t, mut p) = setup();
    let mut store = BodyStore::new(PackWriter::open(&mut p).unwrap());
    let a = put_one(&mut p, &mut store, &noise(30 * 1024, 4));
    store.fsync().unwrap();
    // Replace the stored hash with a wrong one but keep a well-formed frame.
    let path = p.pack_path(1);
    let mut bytes = std::fs::read(&path).unwrap();
    bytes[8] ^= 0xff;
    std::fs::write(&path, &bytes).unwrap();
    let m = corrupt_msg(BodyStore::get(
        &p.reader().unwrap(),
        p.dir(),
        a,
        BodySide::Resp,
    ));
    assert!(m.contains("hash mismatch"), "{m}");
}

#[test]
fn flipped_payload_byte_is_corrupt() {
    let (_t, mut p) = setup();
    let mut store = BodyStore::new(PackWriter::open(&mut p).unwrap());
    let a = put_one(&mut p, &mut store, &noise(30 * 1024, 5));
    store.fsync().unwrap();
    let path = p.pack_path(1);
    let mut bytes = std::fs::read(&path).unwrap();
    let mid = bytes.len() / 2;
    bytes[mid] ^= 0x01;
    std::fs::write(&path, &bytes).unwrap();
    corrupt_msg(BodyStore::get(
        &p.reader().unwrap(),
        p.dir(),
        a,
        BodySide::Resp,
    ));
}

/// Appends a hand-built record to pack 1 and registers it as `ulid`'s body.
fn plant(p: &mut Project, raw_declared: &[u8], hash_of: &[u8], ulid: Ulid) {
    let frame = zstd::bulk::compress(raw_declared, 3).unwrap();
    let hash: [u8; 32] = Sha256::digest(hash_of).into();
    let path = p.pack_path(1);
    let mut f = OpenOptions::new().append(true).open(&path).unwrap();
    let offset = f.metadata().unwrap().len();
    let mut rec = b"CWPK".to_vec();
    rec.extend((frame.len() as u32).to_le_bytes());
    rec.extend(hash);
    rec.extend(&frame);
    f.write_all(&rec).unwrap();
    let tx = p.writer_mut().transaction().unwrap();
    add_exchange(&tx, ulid);
    tx.execute(
        "INSERT INTO blobs (hash, pack_id, offset, len) VALUES (?1, 1, ?2, ?3)",
        rusqlite::params![hash.as_slice(), offset as i64, rec.len() as i64],
    )
    .unwrap();
    tx.execute(
        "INSERT INTO bodies (ulid, side, blob_hash, len) VALUES (?1, 'resp', ?2, ?3)",
        rusqlite::params![ulid.to_string(), hash.as_slice(), raw_declared.len() as i64],
    )
    .unwrap();
    tx.commit().unwrap();
}

#[test]
fn decompression_bomb_refused() {
    let (_t, mut p) = setup();
    let _store = BodyStore::new(PackWriter::open(&mut p).unwrap());
    let id = Ulid::new();
    // 65 MiB of zeros: tiny frame, content size over the 64 MiB cap.
    let big = vec![0u8; 65 * 1024 * 1024];
    plant(&mut p, &big, &big, id);
    let m = corrupt_msg(BodyStore::get(
        &p.reader().unwrap(),
        p.dir(),
        id,
        BodySide::Resp,
    ));
    assert!(m.contains("limit") || m.contains("ratio"), "{m}");
}

#[test]
fn ratio_bomb_refused() {
    let (_t, mut p) = setup();
    let _store = BodyStore::new(PackWriter::open(&mut p).unwrap());
    let id = Ulid::new();
    let big = vec![0u8; 8 * 1024 * 1024];
    plant(&mut p, &big, &big, id);
    let m = corrupt_msg(BodyStore::get(
        &p.reader().unwrap(),
        p.dir(),
        id,
        BodySide::Resp,
    ));
    assert!(m.contains("ratio"), "{m}");
}

#[test]
fn frame_without_content_size_refused() {
    let (_t, mut p) = setup();
    let _store = BodyStore::new(PackWriter::open(&mut p).unwrap());
    let id = Ulid::new();
    let raw = noise(20 * 1024, 6);
    // Stream-compress: the frame header omits the content size.
    let mut enc = zstd::Encoder::new(Vec::new(), 3).unwrap();
    enc.write_all(&raw).unwrap();
    let frame = enc.finish().unwrap();
    let hash: [u8; 32] = Sha256::digest(&raw).into();
    let path = p.pack_path(1);
    let mut rec = b"CWPK".to_vec();
    rec.extend((frame.len() as u32).to_le_bytes());
    rec.extend(hash);
    rec.extend(&frame);
    std::fs::write(&path, &rec).unwrap();
    let tx = p.writer_mut().transaction().unwrap();
    add_exchange(&tx, id);
    tx.execute(
        "INSERT INTO blobs (hash, pack_id, offset, len) VALUES (?1, 1, 0, ?2)",
        rusqlite::params![hash.as_slice(), rec.len() as i64],
    )
    .unwrap();
    tx.execute(
        "INSERT INTO bodies (ulid, side, blob_hash, len) VALUES (?1, 'resp', ?2, ?3)",
        rusqlite::params![id.to_string(), hash.as_slice(), raw.len() as i64],
    )
    .unwrap();
    tx.commit().unwrap();
    corrupt_msg(BodyStore::get(
        &p.reader().unwrap(),
        p.dir(),
        id,
        BodySide::Resp,
    ));
}

#[test]
fn torn_tail_recovery() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("p");
    let mut p = Project::create(&dir).unwrap();
    let mut store = BodyStore::new(PackWriter::open(&mut p).unwrap());
    let bodies: Vec<Vec<u8>> = (0..4)
        .map(|i| noise(20 * 1024 + i, 10 + i as u64))
        .collect();
    let ids: Vec<Ulid> = (0..4).map(|_| Ulid::new()).collect();

    // Three committed bodies, fsynced, committed_len advanced.
    let tx = p.writer_mut().transaction().unwrap();
    for i in 0..3 {
        add_exchange(&tx, ids[i]);
        store.put(&tx, ids[i], BodySide::Resp, &bodies[i]).unwrap();
    }
    store.fsync().unwrap();
    let end3 = store.committed_len();
    Project::set_pack_committed_len(&tx, 1, end3).unwrap();
    tx.commit().unwrap();

    // The fourth is appended and its rows committed, but the pack write is torn.
    let tx = p.writer_mut().transaction().unwrap();
    add_exchange(&tx, ids[3]);
    store.put(&tx, ids[3], BodySide::Resp, &bodies[3]).unwrap();
    tx.commit().unwrap();
    store.fsync().unwrap();
    drop(store);
    let path = p.pack_path(1);
    assert!(std::fs::metadata(&path).unwrap().len() > end3 + 10);
    OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(end3 + 10)
        .unwrap();
    drop(p);

    let mut p = Project::open(&dir).unwrap();
    let report = sweep(&mut p).unwrap();
    assert_eq!(report.packs_checked, 1);
    assert_eq!(report.truncated_bytes, 10);
    assert_eq!(report.exchanges_marked, 1);
    assert_eq!(std::fs::metadata(&path).unwrap().len(), end3);

    let c = p.writer_mut();
    let state: String = c
        .query_row(
            "SELECT state FROM exchanges WHERE ulid = ?1",
            [ids[3].to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "error");
    let (n, kind, from, detail): (i64, String, String, String) = c
        .query_row(
            "SELECT count(*), kind, from_ulid, detail FROM markers",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        (n, kind.as_str(), from, detail.as_str()),
        (1, "gap", ids[3].to_string(), "pack torn")
    );
    let ic: String = c
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(ic, "ok");
    for i in 0..3 {
        assert_eq!(
            BodyStore::get(&p.reader().unwrap(), p.dir(), ids[i], BodySide::Resp)
                .unwrap()
                .unwrap(),
            bodies[i]
        );
    }
    assert!(BodyStore::get(&p.reader().unwrap(), p.dir(), ids[3], BodySide::Resp).is_err());
    // Idempotent.
    let again = sweep(&mut p).unwrap();
    assert_eq!((again.truncated_bytes, again.exchanges_marked), (0, 0));
}

#[test]
fn sweep_survives_garbage_tail_and_deletes_orphans() {
    let (_t, mut p) = setup();
    let mut store = BodyStore::new(PackWriter::open(&mut p).unwrap());
    let a = put_one(&mut p, &mut store, &noise(20 * 1024, 20));
    store.fsync().unwrap();
    let end = store.committed_len();
    let tx = p.writer_mut().transaction().unwrap();
    Project::set_pack_committed_len(&tx, 1, end).unwrap();
    tx.execute(
        "INSERT INTO blobs (hash, pack_id, offset, len, refcount) VALUES (zeroblob(32), 1, 0, 1, 0)",
        [],
    )
    .unwrap();
    tx.commit().unwrap();
    drop(store);
    let path = p.pack_path(1);
    let mut f = OpenOptions::new().append(true).open(&path).unwrap();
    f.write_all(b"CWPK\xff\xff\xff\xffgarbage").unwrap();
    drop(f);
    let r = sweep(&mut p).unwrap();
    assert_eq!(r.truncated_bytes, 15);
    assert_eq!(r.orphans_deleted, 1);
    assert_eq!(r.exchanges_marked, 0);
    assert!(
        BodyStore::get(&p.reader().unwrap(), p.dir(), a, BodySide::Resp)
            .unwrap()
            .is_some()
    );
}

#[test]
fn rotation_opens_next_pack_and_registers_it() {
    let (_t, mut p) = setup();
    let mut w = PackWriter::open(&mut p).unwrap();
    w.set_rotate_limit(100 * 1024);
    let mut store = BodyStore::new(w);
    let mut ids = Vec::new();
    for i in 0..4 {
        ids.push(put_one(&mut p, &mut store, &noise(60 * 1024, 30 + i)));
    }
    let n: i64 = p
        .writer_mut()
        .query_row("SELECT count(*) FROM packs", [], |r| r.get(0))
        .unwrap();
    assert!(n >= 2, "packs: {n}");
    for (i, id) in ids.iter().enumerate() {
        let got = BodyStore::get(&p.reader().unwrap(), p.dir(), *id, BodySide::Resp)
            .unwrap()
            .unwrap();
        assert_eq!(got, noise(60 * 1024, 30 + i as u64));
    }
}

fn committed_three(dir: &std::path::Path) -> (Project, Vec<Ulid>, Vec<Vec<u8>>, Vec<u64>) {
    let mut p = Project::create(dir).unwrap();
    let mut store = BodyStore::new(PackWriter::open(&mut p).unwrap());
    let mut ids = Vec::new();
    let mut bodies = Vec::new();
    let mut ends = Vec::new();
    for i in 0..3u64 {
        let b = noise(20 * 1024, 100 + i);
        ids.push(put_one(&mut p, &mut store, &b));
        bodies.push(b);
        ends.push(store.committed_len());
    }
    store.fsync().unwrap();
    let tx = p.writer_mut().transaction().unwrap();
    Project::set_pack_committed_len(&tx, 1, 0).unwrap();
    tx.commit().unwrap();
    (p, ids, bodies, ends)
}

fn state_of(p: &mut Project, id: Ulid) -> String {
    p.writer_mut()
        .query_row(
            "SELECT state FROM exchanges WHERE ulid = ?1",
            [id.to_string()],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn sweep_repairs_index_after_crash_between_truncate_and_cleanup() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut p, ids, _b, ends) = committed_three(&tmp.path().join("p"));
    // A previous sweep truncated the file but died before the database cleanup.
    OpenOptions::new()
        .write(true)
        .open(p.pack_path(1))
        .unwrap()
        .set_len(ends[1])
        .unwrap();
    let r = sweep(&mut p).unwrap();
    assert_eq!(r.truncated_bytes, 0);
    assert_eq!(r.exchanges_marked, 1);
    assert_eq!(state_of(&mut p, ids[2]), "error");
    assert_eq!(state_of(&mut p, ids[1]), "complete");
    let n: i64 = p
        .writer_mut()
        .query_row("SELECT count(*) FROM blobs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 2);
}

#[test]
fn mid_file_bad_record_loses_everything_after_it() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut p, ids, bodies, ends) = committed_three(&tmp.path().join("p"));
    let path = p.pack_path(1);
    let mut bytes = std::fs::read(&path).unwrap();
    bytes[ends[0] as usize] ^= 0xff; // magic of record 2
    std::fs::write(&path, bytes).unwrap();
    let r = sweep(&mut p).unwrap();
    assert_eq!(std::fs::metadata(&path).unwrap().len(), ends[0]);
    assert_eq!(r.exchanges_marked, 2);
    assert_eq!(state_of(&mut p, ids[0]), "complete");
    assert_eq!(state_of(&mut p, ids[1]), "error");
    assert_eq!(state_of(&mut p, ids[2]), "error");
    let got = BodyStore::get(&p.reader().unwrap(), p.dir(), ids[0], BodySide::Resp)
        .unwrap()
        .unwrap();
    assert_eq!(got, bodies[0]);
}

#[test]
fn missing_pack_file_loses_its_blobs() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut p, ids, _b, _e) = committed_three(&tmp.path().join("p"));
    std::fs::remove_file(p.pack_path(1)).unwrap();
    let r = sweep(&mut p).unwrap();
    assert_eq!(r.exchanges_marked, 3);
    for id in ids {
        assert_eq!(state_of(&mut p, id), "error");
    }
}

#[test]
fn committed_len_beyond_file_is_handled() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut p, ids, _b, ends) = committed_three(&tmp.path().join("p"));
    let tx = p.writer_mut().transaction().unwrap();
    Project::set_pack_committed_len(&tx, 1, ends[2] + 1000).unwrap();
    tx.commit().unwrap();
    OpenOptions::new()
        .write(true)
        .open(p.pack_path(1))
        .unwrap()
        .set_len(ends[1] + 5)
        .unwrap();
    let r = sweep(&mut p).unwrap();
    assert_eq!(r.exchanges_marked, 1);
    assert_eq!(state_of(&mut p, ids[2]), "error");
    assert_eq!(state_of(&mut p, ids[0]), "complete");
}

#[test]
fn missing_exchange_is_counted_without_marker() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut p, ids, _b, ends) = committed_three(&tmp.path().join("p"));
    p.writer_mut()
        .execute(
            "DELETE FROM exchanges WHERE ulid = ?1",
            [ids[2].to_string()],
        )
        .unwrap();
    OpenOptions::new()
        .write(true)
        .open(p.pack_path(1))
        .unwrap()
        .set_len(ends[1])
        .unwrap();
    let r = sweep(&mut p).unwrap();
    assert_eq!((r.exchanges_marked, r.missing_exchanges), (0, 1));
    let n: i64 = p
        .writer_mut()
        .query_row("SELECT count(*) FROM markers", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0);
}

#[test]
fn set_committed_len_requires_a_row() {
    let (_t, mut p) = setup();
    let tx = p.writer_mut().transaction().unwrap();
    assert!(Project::set_pack_committed_len(&tx, 9, 1).is_err());
}

#[cfg(unix)]
#[test]
fn symlinked_pack_is_refused_and_never_touched() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut p, ids, _b, _e) = committed_three(&tmp.path().join("p"));
    let victim = tmp.path().join("victim");
    std::fs::write(&victim, vec![7u8; 5000]).unwrap();
    std::fs::remove_file(p.pack_path(1)).unwrap();
    std::os::unix::fs::symlink(&victim, p.pack_path(1)).unwrap();
    assert!(matches!(sweep(&mut p), Err(StoreError::Corrupt(_))));
    assert_eq!(std::fs::metadata(&victim).unwrap().len(), 5000);
    let r = BodyStore::get(&p.reader().unwrap(), p.dir(), ids[0], BodySide::Resp);
    assert!(matches!(r, Err(StoreError::Corrupt(_))), "{r:?}");
    assert!(PackWriter::open(&mut p).is_err());
}

#[cfg(unix)]
#[test]
fn symlinked_packs_dir_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut p, _ids, _b, _e) = committed_three(&tmp.path().join("p"));
    let real = tmp.path().join("p").join("packs");
    let moved = tmp.path().join("elsewhere");
    std::fs::rename(&real, &moved).unwrap();
    std::os::unix::fs::symlink(&moved, &real).unwrap();
    assert!(matches!(sweep(&mut p), Err(StoreError::Corrupt(_))));
}

#[test]
fn missing_pack_does_not_touch_other_packs() {
    let (_t, mut p) = setup();
    let mut w = PackWriter::open(&mut p).unwrap();
    w.set_rotate_limit(100 * 1024);
    let mut store = BodyStore::new(w);
    let bodies: Vec<Vec<u8>> = (0..4).map(|i| noise(60 * 1024, 200 + i)).collect();
    let ids: Vec<Ulid> = bodies
        .iter()
        .map(|b| put_one(&mut p, &mut store, b))
        .collect();
    store.fsync().unwrap();
    // Which pack holds each body.
    let pack_of = |p: &mut Project, id: Ulid| -> i64 {
        p.writer_mut()
            .query_row(
                "SELECT pack_id FROM blobs WHERE hash = \
                 (SELECT blob_hash FROM bodies WHERE ulid = ?1)",
                [id.to_string()],
                |r| r.get(0),
            )
            .unwrap()
    };
    let packs: Vec<i64> = ids.iter().map(|i| pack_of(&mut p, *i)).collect();
    assert!(
        packs.contains(&1) && packs.iter().any(|&x| x >= 2),
        "{packs:?}"
    );
    std::fs::remove_file(p.pack_path(1)).unwrap();
    sweep(&mut p).unwrap();
    for (i, id) in ids.iter().enumerate() {
        let st = state_of(&mut p, *id);
        let got = BodyStore::get(&p.reader().unwrap(), p.dir(), *id, BodySide::Resp);
        if packs[i] == 1 {
            assert_eq!(st, "error");
            assert!(got.is_err());
        } else {
            assert_eq!(st, "complete", "body {i} in pack {}", packs[i]);
            assert_eq!(got.unwrap().unwrap(), bodies[i]);
        }
    }
}
