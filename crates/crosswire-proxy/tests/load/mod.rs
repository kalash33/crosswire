//! In-process HTTP/1.1 load harness shared by the §13 performance gates.
//!
//! It provides three pieces the gate binaries compose:
//!
//! * [`spawn_echo_upstream`] — a keep-alive HTTP/1.1 origin that returns a
//!   fixed-size body for every request.
//! * [`drive`] — opens `conns` keep-alive TCP connections that each issue
//!   requests back-to-back for a fixed duration, either straight to the
//!   upstream ([`Target::Direct`]) or through a running proxy in absolute
//!   form ([`Target::ViaProxy`]), recording per-request wall-clock latency.
//! * [`Stats`] / [`percentiles`] — a nearest-rank p50/p99 reducer over the
//!   merged per-connection latency samples, plus total requests and
//!   requests-per-second.
//!
//! The harness never starts a proxy itself: a gate task spawns one with
//! [`crate::common::ProxyHarness`] and passes its address to
//! [`Target::ViaProxy`]. This module is pulled into each gate binary with
//! `#[path = "load/mod.rs"] mod load;`, so not every item is used by every
//! binary; `dead_code` is allowed accordingly.
#![allow(dead_code)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Merged latency and throughput for one [`drive`] run.
#[derive(Clone, Copy, Debug)]
pub struct Stats {
    /// Median per-request latency, microseconds (nearest-rank).
    pub p50_us: u64,
    /// 99th-percentile per-request latency, microseconds (nearest-rank).
    pub p99_us: u64,
    /// Total requests that completed across every connection.
    pub reqs: u64,
    /// Wall-clock time the drive actually ran.
    pub elapsed: Duration,
}

impl Stats {
    /// Completed requests per second over [`Stats::elapsed`]; `0.0` when no
    /// time elapsed.
    pub fn reqs_per_sec(&self) -> f64 {
        let secs = self.elapsed.as_secs_f64();
        if secs > 0.0 {
            self.reqs as f64 / secs
        } else {
            0.0
        }
    }
}

/// Where [`drive`] sends its requests.
pub enum Target {
    /// Connect straight to this upstream address (origin-form requests).
    Direct(SocketAddr),
    /// Connect to a running proxy and send absolute-form requests for
    /// `upstream_authority` (e.g. `"u.test:8080"`).
    ViaProxy {
        /// The proxy's listening address.
        proxy: SocketAddr,
        /// The upstream authority the proxy should relay to.
        upstream_authority: String,
    },
}

/// Nearest-rank p50 and p99 (microseconds) over `samples`, which is sorted
/// in place. Returns `(0, 0)` for an empty set.
pub fn percentiles(samples: &mut [u64]) -> (u64, u64) {
    if samples.is_empty() {
        return (0, 0);
    }
    samples.sort_unstable();
    (nearest_rank(samples, 0.50), nearest_rank(samples, 0.99))
}

/// The nearest-rank value at `q` (0.0..=1.0) of an already-sorted,
/// non-empty slice.
fn nearest_rank(sorted: &[u64], q: f64) -> u64 {
    let n = sorted.len();
    // rank = ceil(q * n), clamped to 1..=n, then 0-based.
    let rank = (q * n as f64).ceil() as usize;
    let idx = rank.clamp(1, n) - 1;
    sorted[idx]
}

/// Spawns a keep-alive HTTP/1.1 upstream on an ephemeral loopback port that
/// answers every request with a `body_len`-byte body (`Content-Length`
/// set). Each request is read fully before the response is written.
/// Returns the bound address; the server runs until the process exits.
pub async fn spawn_echo_upstream(body_len: usize) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    // Pre-render the full response once; every reply is a copy of it.
    let response: Arc<[u8]> = {
        let mut r = format!("HTTP/1.1 200 OK\r\nContent-Length: {body_len}\r\n\r\n").into_bytes();
        r.resize(r.len() + body_len, b'a');
        r.into()
    };
    tokio::spawn(async move {
        loop {
            let Ok((conn, _)) = listener.accept().await else {
                break;
            };
            let _ = conn.set_nodelay(true);
            tokio::spawn(serve_echo(conn, Arc::clone(&response)));
        }
    });
    addr
}

/// Serves one keep-alive connection: read each complete request head (and
/// any `Content-Length` body) then write the fixed response, until the peer
/// closes. Requests here are GETs (no body), but a `Content-Length` body is
/// drained so the loop frames back-to-back pipelined requests correctly.
async fn serve_echo(mut conn: TcpStream, response: Arc<[u8]>) {
    let mut carry: Vec<u8> = Vec::with_capacity(16 * 1024);
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        // Ensure a full head (through the blank line) is buffered.
        let head_end = loop {
            if let Some(pos) = find_subslice(&carry, b"\r\n\r\n") {
                break pos + 4;
            }
            match conn.read(&mut buf).await {
                Ok(0) | Err(_) => return,
                Ok(n) => carry.extend_from_slice(&buf[..n]),
            }
        };
        // Drain a declared request body, if any, so framing stays aligned.
        let body_len = content_length(&carry[..head_end]).unwrap_or(0);
        let total = head_end + body_len;
        while carry.len() < total {
            match conn.read(&mut buf).await {
                Ok(0) | Err(_) => return,
                Ok(n) => carry.extend_from_slice(&buf[..n]),
            }
        }
        carry.drain(..total);
        if conn.write_all(&response).await.is_err() {
            return;
        }
    }
}

/// Parses a `Content-Length` value from a request head, if present.
fn content_length(head: &[u8]) -> Option<usize> {
    let text = std::str::from_utf8(head).ok()?;
    for line in text.split("\r\n") {
        let (name, value) = line.split_once(':')?;
        if name.eq_ignore_ascii_case("content-length") {
            return value.trim().parse().ok();
        }
    }
    None
}

/// Finds the first occurrence of `needle` in `haystack`.
fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Opens `conns` keep-alive connections to `target`, each issuing requests
/// back-to-back for `duration`, and returns the merged latency percentiles,
/// total requests, and elapsed time.
///
/// `body_len` is the size of the upstream **response** body being relayed;
/// the requests themselves are bodyless GETs. For [`Target::Direct`] the
/// connection goes straight to the upstream in origin form; for
/// [`Target::ViaProxy`] it goes to the proxy in absolute form.
pub async fn drive(
    target: Target,
    conns: usize,
    duration: Duration,
    body_len: usize,
) -> Stats {
    // Resolve the connect address and the request line each connection sends.
    let (connect_addr, request) = match &target {
        Target::Direct(addr) => {
            let req = b"GET / HTTP/1.1\r\nHost: upstream.load\r\n\r\n".to_vec();
            (*addr, req)
        }
        Target::ViaProxy {
            proxy,
            upstream_authority,
        } => {
            // Absolute-form request: the proxy relays it to the authority.
            let host = upstream_authority
                .rsplit_once(':')
                .map_or(upstream_authority.as_str(), |(h, _)| h);
            let req = format!(
                "GET http://{upstream_authority}/ HTTP/1.1\r\nHost: {host}\r\n\r\n"
            )
            .into_bytes();
            (*proxy, req)
        }
    };
    let request: Arc<[u8]> = request.into();

    // The expected response: a 200 with exactly `body_len` body bytes.
    let want_body = body_len;

    let start = Instant::now();
    let deadline = start + duration;

    let mut handles = Vec::with_capacity(conns);
    for _ in 0..conns {
        let request = Arc::clone(&request);
        handles.push(tokio::spawn(async move {
            drive_one(connect_addr, &request, want_body, deadline).await
        }));
    }

    let mut all: Vec<u64> = Vec::new();
    for h in handles {
        if let Ok(samples) = h.await {
            all.extend(samples);
        }
    }
    let elapsed = start.elapsed();

    let reqs = all.len() as u64;
    let (p50_us, p99_us) = percentiles(&mut all);
    Stats {
        p50_us,
        p99_us,
        reqs,
        elapsed,
    }
}

/// Runs one keep-alive connection until `deadline`, returning the per-request
/// latencies (microseconds). A connection error ends that connection's loop
/// and returns whatever it had measured.
async fn drive_one(
    addr: SocketAddr,
    request: &[u8],
    want_body: usize,
    deadline: Instant,
) -> Vec<u64> {
    let mut latencies: Vec<u64> = Vec::new();
    let Ok(mut conn) = TcpStream::connect(addr).await else {
        return latencies;
    };
    let _ = conn.set_nodelay(true);

    let mut carry: Vec<u8> = Vec::with_capacity(want_body + 256);
    let mut buf = vec![0u8; 64 * 1024];

    while Instant::now() < deadline {
        let sent = Instant::now();
        if conn.write_all(request).await.is_err() {
            break;
        }
        // Read one full response: head through blank line, then `want_body`.
        if read_one_response(&mut conn, &mut carry, &mut buf, want_body)
            .await
            .is_none()
        {
            break;
        }
        latencies.push(sent.elapsed().as_micros() as u64);
    }
    latencies
}

/// Consumes exactly one HTTP/1.1 response (head + `want_body` body bytes)
/// from `conn`, buffering leftovers in `carry`. Returns `Some(())` on a
/// framed response, or `None` on EOF/error.
async fn read_one_response(
    conn: &mut TcpStream,
    carry: &mut Vec<u8>,
    buf: &mut [u8],
    want_body: usize,
) -> Option<()> {
    let head_end = loop {
        if let Some(pos) = find_subslice(carry, b"\r\n\r\n") {
            break pos + 4;
        }
        match conn.read(buf).await {
            Ok(0) | Err(_) => return None,
            Ok(n) => carry.extend_from_slice(&buf[..n]),
        }
    };
    // The echo upstream always frames with Content-Length; trust the wanted
    // size, but fall back to any declared length so relayed responses frame.
    let body_len = content_length(&carry[..head_end]).unwrap_or(want_body);
    let total = head_end + body_len;
    while carry.len() < total {
        match conn.read(buf).await {
            Ok(0) | Err(_) => return None,
            Ok(n) => carry.extend_from_slice(&buf[..n]),
        }
    }
    carry.drain(..total);
    Some(())
}
