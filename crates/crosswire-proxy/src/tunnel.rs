//! Opaque byte tunnels (SNI passthrough) and their one recorded exchange.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crosswire_client::Conn;
use crosswire_model::{
    Body, Exchange, Fidelity, FramingRecord, Head, Message, MessageKind, TunnelReason,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::record::RecordCtx;

/// Copy buffer per direction.
const BUF: usize = 64 * 1024;

/// What a tunnel carried.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TunnelStats {
    /// Bytes relayed client to upstream.
    pub(crate) bytes_in: u64,
    /// Bytes relayed upstream to client.
    pub(crate) bytes_out: u64,
    /// How long the tunnel was open.
    pub(crate) duration: Duration,
}

/// Copies `from` to `to` until EOF (then half-closes `to`) or an error,
/// counting bytes into `count` and stamping `last` (ms since `start`).
async fn pump<R, W>(
    from: &mut R,
    to: &mut W,
    count: &AtomicU64,
    last: &AtomicU64,
    start: Instant,
) -> std::io::Result<()>
where
    R: AsyncRead + Unpin + ?Sized,
    W: AsyncWrite + Unpin + ?Sized,
{
    let mut buf = vec![0u8; BUF];
    loop {
        let n = from.read(&mut buf).await?;
        if n == 0 {
            // Half-close; the other direction may still be talking.
            let _ = to.shutdown().await;
            return Ok(());
        }
        to.write_all(&buf[..n]).await?;
        to.flush().await?;
        count.fetch_add(n as u64, Ordering::Relaxed);
        last.store(millis(start.elapsed()), Ordering::Relaxed);
    }
}

fn millis(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

/// Resolves once neither direction has moved for `idle`.
async fn idle_watch(idle: Duration, last: &AtomicU64, start: Instant) {
    loop {
        let quiet_since = Duration::from_millis(last.load(Ordering::Relaxed));
        let deadline = start + quiet_since + idle;
        if Instant::now() >= deadline {
            return;
        }
        tokio::time::sleep_until(deadline.into()).await;
    }
}

/// Relays bytes between `a` (the client) and `b` (the upstream) until both
/// directions end, either errors, or nothing moves for `idle`. Then records
/// one [`Head::Tunnel`] exchange through `ctx` (never awaited) and returns
/// the counts.
pub(crate) async fn run(
    a: Box<dyn Conn>,
    b: Box<dyn Conn>,
    reason: TunnelReason,
    idle: Option<Duration>,
    ctx: RecordCtx,
) -> TunnelStats {
    let start = Instant::now();
    let (bytes_in, bytes_out, last) = (AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0));
    {
        let (mut ar, mut aw) = tokio::io::split(a);
        let (mut br, mut bw) = tokio::io::split(b);
        let both = async {
            tokio::try_join!(
                pump(&mut ar, &mut bw, &bytes_in, &last, start),
                pump(&mut br, &mut aw, &bytes_out, &last, start),
            )
        };
        let quiet = async {
            match idle {
                Some(idle) => idle_watch(idle, &last, start).await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            res = both => if let Err(e) = res {
                tracing::debug!(error = %e, "tunnel ended with an error");
            },
            () = quiet => tracing::debug!("tunnel idle; closing"),
        }
        // Dropping the halves closes both sockets.
    }
    let stats = TunnelStats {
        bytes_in: bytes_in.into_inner(),
        bytes_out: bytes_out.into_inner(),
        duration: start.elapsed(),
    };
    let head = Head::Tunnel {
        bytes_in: stats.bytes_in,
        bytes_out: stats.bytes_out,
        duration_ms: millis(stats.duration),
        reason,
    };
    let msg = Message::new(
        MessageKind::Tunnel,
        Fidelity::Opaque,
        head,
        FramingRecord::None,
        Body::None,
        ctx.info.clone(),
    );
    ctx.submit(Exchange::new(msg));
    stats
}
