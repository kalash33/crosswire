//! Every stored body hash is SHA-256 of the submitted bytes, computed once
//! and written identically everywhere it appears: the pack record header,
//! `blobs.hash`, `bodies.blob_hash` and the row JSON's `blob.hash`.

use bytes::Bytes;
use crosswire_model::{
    Body, Capture, ConnectionInfo, Exchange, Fidelity, FramingRecord, Head, Message, MessageKind,
};
use crosswire_store::{BodySide, BodyStore, Project, Store, StoreConfig};
use rusqlite::OptionalExtension;
use sha2::{Digest, Sha256};
use std::io::{Read, Seek, SeekFrom};
use std::time::Duration;
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

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn msg(body: Body) -> Message {
    Message::new(
        MessageKind::Normalized,
        Fidelity::Normalized,
        Head::Normalized { map: Vec::new() },
        FramingRecord::None,
        body,
        ConnectionInfo::new("example.test", 443),
    )
}

/// One corpus entry: the response body and, where known, the SHA-256
/// computed outside Rust (`shasum -a 256`).
struct Case {
    resp: Body,
    raw: Vec<u8>,
    literal: Option<&'static str>,
}

fn case(raw: Vec<u8>, literal: Option<&'static str>) -> Case {
    Case {
        resp: Body::inline(Bytes::from(raw.clone())),
        raw,
        literal,
    }
}

fn corpus() -> Vec<Case> {
    let relay = Capture {
        len: 1 << 20,
        sha256: [0x5a; 32],
    };
    let with = |raw: Vec<u8>, truncated_at, capture_dropped, relay| Case {
        resp: Body::Inline {
            bytes: Bytes::from(raw.clone()),
            truncated_at,
            capture_dropped,
            relay,
        },
        raw,
        literal: None,
    };
    vec![
        // printf a | shasum -a 256
        case(
            b"a".to_vec(),
            Some("ca978112ca1bbdcafac231b39a23dc4da786eff8147c4e72b9807785afee48bb"),
        ),
        case(vec![b'b'; 16 * 1024], None),
        // 16 KiB + 1 of 'a' (the first pack-sized body)
        case(
            vec![b'a'; 16 * 1024 + 1],
            Some("608b801060d579a7b659a01755259a13dec8853b97d3d8062763c97ffac6ddae"),
        ),
        // A deduplicated pair.
        case(noise(100_000, 7), None),
        case(noise(100_000, 7), None),
        with(noise(40_000, 8), Some(40_000), false, Some(relay)),
        with(noise(30_000, 9), None, true, Some(relay)),
        with(noise(20_000, 10), None, false, Some(relay)),
        case(noise(8 * 1024 * 1024, 11), None),
        // head -c 1048576 /dev/zero | shasum -a 256
        case(
            vec![0; 1024 * 1024],
            Some("30e14955ebf1352266dc2ff8067e68104607e750abb9d3b36582b8af909fcb58"),
        ),
    ]
}

fn blob_hash(body: &Body) -> [u8; 32] {
    match body {
        Body::Blob { hash, .. } => *hash,
        other => panic!("stored body is not a blob reference: {other:?}"),
    }
}

/// The 32-byte hash in the record header at `offset` of pack `pack_id`.
fn record_header_hash(p: &Project, pack_id: u32, offset: u64) -> [u8; 32] {
    let mut f = std::fs::File::open(p.pack_path(pack_id)).unwrap();
    f.seek(SeekFrom::Start(offset)).unwrap();
    let mut head = [0u8; 40];
    f.read_exact(&mut head).unwrap();
    assert_eq!(&head[..4], b"CWPK");
    head[8..].try_into().unwrap()
}

#[test]
fn every_stored_hash_is_one_sha256_of_the_submitted_bytes() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("p");
    let store = Store::open(
        Project::create(&dir).unwrap(),
        StoreConfig {
            batch_interval: Duration::from_millis(20),
            ..StoreConfig::default()
        },
    )
    .unwrap();
    let cases = corpus();
    let mut ids: Vec<Ulid> = Vec::new();
    for c in &cases {
        let mut ex = Exchange::new(msg(Body::None));
        ex.set_response(msg(c.resp.clone()));
        ids.push(ex.id());
        store.sink().submit(ex).unwrap();
    }
    store.close().unwrap();

    let p = Project::open(&dir).unwrap();
    let conn = p.reader().unwrap();
    for (c, id) in cases.iter().zip(&ids) {
        let want: [u8; 32] = Sha256::digest(&c.raw).into();
        if let Some(lit) = c.literal {
            assert_eq!(hex(&want), lit, "independent SHA-256 of case {id}");
        }
        let json: String = conn
            .query_row(
                "SELECT json FROM exchanges WHERE ulid = ?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        let ex: Exchange = serde_json::from_str(&json).unwrap();
        let resp = ex.response().unwrap().body();
        assert_eq!(blob_hash(resp), want, "JSON blob.hash of {id}");
        if let (
            Body::Blob {
                len,
                truncated_at,
                capture_dropped,
                relay,
                ..
            },
            Body::Inline {
                truncated_at: t,
                capture_dropped: d,
                relay: r,
                ..
            },
        ) = (resp, &c.resp)
        {
            assert_eq!(*len, c.raw.len() as u64);
            assert_eq!((truncated_at, capture_dropped, relay), (t, d, r));
        }
        let blob_ref: Option<Vec<u8>> = conn
            .query_row(
                "SELECT blob_hash FROM bodies WHERE ulid = ?1 AND side = 'resp'",
                [id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        if c.raw.len() > 16 * 1024 {
            let blob_ref = blob_ref.expect("a pack body has a blob_hash");
            assert_eq!(blob_ref, want, "bodies.blob_hash of {id}");
            let (hash, pack_id, offset): (Vec<u8>, u32, u64) = conn
                .query_row(
                    "SELECT hash, pack_id, offset FROM blobs WHERE hash = ?1",
                    [want.as_slice()],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()
                .unwrap()
                .expect("blobs row");
            assert_eq!(hash, want, "blobs.hash of {id}");
            assert_eq!(
                record_header_hash(&p, pack_id, offset),
                want,
                "record header hash of {id}"
            );
        } else {
            assert_eq!(blob_ref, None, "an inline body has no blob_hash");
        }
        // Reads decompress and verify the record's hash (`read_record`).
        let back = BodyStore::get(&conn, p.dir(), *id, BodySide::Resp)
            .unwrap()
            .unwrap();
        assert!(back == c.raw, "body of {id} reads back");
    }
    // The pair was stored once, referenced twice.
    let dup: [u8; 32] = Sha256::digest(noise(100_000, 7)).into();
    let (refcount, blobs): (i64, i64) = conn
        .query_row(
            "SELECT (SELECT refcount FROM blobs WHERE hash = ?1), (SELECT count(*) FROM blobs)",
            [dup.as_slice()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(refcount, 2);
    let pack_bodies = cases.iter().filter(|c| c.raw.len() > 16 * 1024).count();
    assert_eq!(blobs, i64::try_from(pack_bodies - 1).unwrap());
}
