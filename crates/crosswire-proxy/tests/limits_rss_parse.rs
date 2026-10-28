//! Flat-RSS proof for the proxy's parse-limit rows (§13 gate #18): an
//! attacker streams far more head / headers / chunks / trailer than the
//! `ParseLimits` row allows; the proxy must refuse after `limit + tail` and
//! its anonymous RSS must not grow toward the volume sent.
//!
//! Its own test binary: RSS is process-wide, and `with_flat_rss` serializes
//! every measured section, so each test runs its whole rig inside it.
//!
//! Every test is `multi_thread`: on a current-thread runtime a CPU-bound
//! drive never yields to the background sampler, the peak would equal the
//! baseline, and the assertion would pass vacuously. The hostile input is
//! streamed from ONE small reused buffer so the test itself never holds
//! the volume it claims to send (RSS here is this process's, proxy and
//! test together).

mod common;
#[path = "flat_rss/mod.rs"]
mod flat_rss;

use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use common::relay::wait_for;
use common::upstream::spawn_raw;
use common::*;
use crosswire_h1::{ParseLimits, ParserConfig};
use crosswire_model::MarkerKind;
use crosswire_proxy::{NoopHandler, ProxyLimits};
use flat_rss::with_flat_rss;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const MIB: u64 = 1024 * 1024;
/// How much hostile input each row attempts to send (the proxy refuses far
/// earlier; the write loop ends when it closes the connection).
const ATTEMPT: u64 = 256 * MIB;
const OK: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok";
const ESTABLISHED: &[u8] = b"HTTP/1.1 200 Connection established\r\n\r\n";
const CHUNKED_HEAD: &[u8] =
    b"POST /up HTTP/1.1\r\nHost: u.test\r\nTransfer-Encoding: chunked\r\n\r\n";

/// Streams `prefix` then `filler` repeated, in ~64 KiB writes from one
/// reused buffer, until `ATTEMPT` bytes are sent or the proxy closes the
/// connection (the expected refusal). Returns the bytes actually written and
/// asserts the proxy raised a `LimitExceeded` marker containing `what`.
async fn stream_refused(prefix: &[u8], filler: &[u8], what: &str) -> u64 {
    let up = spawn_raw(vec![OK.to_vec()], None).await;
    let port = up.port();
    let mut cfg = loopback_config();
    cfg.limits = ProxyLimits::builder()
        .parse(
            ParserConfig::builder()
                .limits(ParseLimits::builder().build())
                .build(),
        )
        .build();
    let h = ProxyHarness::start_wired(
        cfg,
        Arc::new(NoopHandler),
        UpstreamWiring {
            extra_roots: Vec::new(),
            allow: vec![("127.0.0.1".to_string(), port)],
            resolve: vec![("u.test".to_string(), IpAddr::from([127, 0, 0, 1]))],
            ..UpstreamWiring::default()
        },
    )
    .await;

    let mut s = connect_raw(h.addr).await;
    let authority = format!("u.test:{port}");
    s.write_all(format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n\r\n").as_bytes())
        .await
        .unwrap();
    assert_eq!(read_until(&mut s, b"\r\n\r\n").await, ESTABLISHED);

    let (mut rd, mut wr) = s.into_split();
    // Drain the proxy's reply concurrently so it never blocks on us.
    let reader = tokio::spawn(async move {
        let mut buf = [0u8; 4096];
        while matches!(rd.read(&mut buf).await, Ok(n) if n > 0) {}
    });

    // One reusable buffer of whole filler units.
    let mut block = Vec::with_capacity(64 * 1024 + filler.len());
    while block.len() < 64 * 1024 {
        block.extend_from_slice(filler);
    }

    let mut sent = 0u64;
    let drive = async {
        if wr.write_all(prefix).await.is_err() {
            return;
        }
        sent += prefix.len() as u64;
        while sent < ATTEMPT {
            if wr.write_all(&block).await.is_err() {
                return; // closed by the proxy: the bound working
            }
            sent += block.len() as u64;
        }
    };
    tokio::time::timeout(Duration::from_secs(60), drive)
        .await
        .expect("the proxy never refused the hostile stream");
    drop(wr);
    let _ = tokio::time::timeout(Duration::from_secs(10), reader).await;

    wait_for(&format!("a LimitExceeded marker with {what:?}"), || {
        h.sink
            .markers
            .lock()
            .unwrap()
            .iter()
            .any(|m| m.kind() == MarkerKind::LimitExceeded && m.detail().contains(what))
    })
    .await;
    assert!(
        sent < ATTEMPT,
        "the proxy consumed all {sent} bytes instead of refusing early"
    );
    sent
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn head_size_flat_rss() {
    // 8000-byte lines: under the 16 KiB line limit and far under 256
    // headers, so the 64 KiB head-size row is what trips.
    let mut filler = b"X-Pad: ".to_vec();
    filler.extend(std::iter::repeat_n(b'a', 8000));
    filler.extend_from_slice(b"\r\n");
    with_flat_rss(24, || async {
        stream_refused(
            b"GET / HTTP/1.1\r\nHost: u.test\r\n",
            &filler,
            "message head too large",
        )
        .await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn header_count_flat_rss() {
    // The count row is checked when a head completes (an unterminated head
    // is bounded by head size instead), so send a complete 400-header head
    // and then keep streaming junk: the proxy must refuse and stop reading.
    let mut prefix = b"GET / HTTP/1.1\r\nHost: u.test\r\n".to_vec();
    for _ in 0..400 {
        prefix.extend_from_slice(b"X-H: v\r\n");
    }
    prefix.extend_from_slice(b"\r\n");
    with_flat_rss(24, || async {
        stream_refused(&prefix, b"GET / HTTP/1.1\r\n", "too many headers").await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn line_length_flat_rss() {
    // As with the count row, the line limit is checked when the head
    // completes: a complete head with one 20000-byte line (over 16 KiB, under
    // the 64 KiB head size), then junk streamed after the refusal.
    let mut prefix = b"GET / HTTP/1.1\r\nHost: u.test\r\nX-Long: ".to_vec();
    prefix.extend(std::iter::repeat_n(b'b', 20_000));
    prefix.extend_from_slice(b"\r\n\r\n");
    with_flat_rss(24, || async {
        stream_refused(&prefix, b"GET / HTTP/1.1\r\n", "line too long").await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn max_chunks_flat_rss() {
    // Millions of one-byte chunks; the 1M-chunk row trips. The body up to
    // the limit is legitimately relayed and recorded (about 6 MB upstream
    // plus the recorded copy), so this cap sits above `limit + tail`.
    with_flat_rss(64, || async {
        stream_refused(CHUNKED_HEAD, b"1\r\nx\r\n", "too many chunks").await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chunk_size_line_flat_rss() {
    // A chunk-size line with an endless extension.
    with_flat_rss(24, || async {
        let mut prefix = CHUNKED_HEAD.to_vec();
        prefix.extend_from_slice(b"5;ext=");
        stream_refused(&prefix, b"cccccccccccccccc", "line too long").await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trailer_size_flat_rss() {
    // A valid terminal chunk, then a trailer section that never ends.
    with_flat_rss(24, || async {
        let mut prefix = CHUNKED_HEAD.to_vec();
        prefix.extend_from_slice(b"0\r\n");
        stream_refused(&prefix, b"X-T: v\r\n", "trailer section too large").await;
    })
    .await;
}
