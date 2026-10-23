//! Moves one message from one connection to another through an h1 parser,
//! forwarding exactly the bytes the parser consumed.

use std::pin::Pin;
use std::task::Poll;
use std::time::{Duration, Instant};

use bytes::{Buf, Bytes, BytesMut};
use crosswire_h1::{ParseError, Parser, Progress, RawMessage};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};

use crate::capture::CaptureBuf;
use crate::Decision;

/// Size of one read: the most bytes read ahead of the parser.
pub(crate) const READ_BUF: usize = 64 * 1024;

/// How a body pump ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PumpEnd {
    /// The message is complete; bytes after it stay pending (pipelining).
    Done,
    /// The source closed, which completed an until-close body.
    Eof,
    /// The handler dropped the message.
    Dropped,
}

/// Which clock ran out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Clock {
    /// Nothing moved for the idle limit.
    Idle,
    /// The exchange's total deadline passed.
    Total,
}

/// Why a pump stopped before the message was complete.
#[derive(Debug)]
pub(crate) enum PumpFail {
    /// The source's bytes do not parse. For a body, every byte consumed
    /// before the error was forwarded; for a head, the bytes consumed are
    /// in [`Pump::head_seen`]. The rest is still pending.
    Parse(ParseError),
    /// The source closed mid-message; the parser's verdict on that.
    Eof(ParseError),
    /// Reading the source failed.
    Read(std::io::Error),
    /// Writing the destination failed.
    Write(std::io::Error),
    /// A clock ran out.
    Timeout(Clock),
}

/// What an idle connection holds before the next message is sent on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Probe {
    /// Nothing to read: the connection can carry the next exchange.
    Idle,
    /// The peer closed it (or it failed).
    Closed,
    /// Bytes nobody asked for: they belong to no exchange.
    Stray,
}

/// When the next read or write must finish, and which clock that is.
type Limit = Option<(Instant, Clock)>;

/// One direction of a connection: unconsumed bytes waiting to be
/// re-presented to the parser, and when the first byte of a read arrived.
///
/// Bounded buffering: a read happens only when the parser asks for more
/// (`NeedMore`), and the consumed bytes are written to the destination,
/// with the write awaited, before that read. At most [`READ_BUF`] bytes
/// plus one incomplete body token are held.
///
/// Every read and write is bounded by the per-operation `idle` limit and
/// the `deadline`, when set ([`Pump::set_limits`]).
pub(crate) struct Pump {
    pending: BytesMut,
    scratch: Box<[u8]>,
    first_byte: Option<Instant>,
    /// The bytes the parser consumed for the current head (at most the
    /// head size limit): forwarded as-is when the head does not parse.
    head_seen: BytesMut,
    idle: Option<Duration>,
    deadline: Option<Instant>,
}

impl Pump {
    /// A pump whose first bytes are `initial` (already read elsewhere).
    pub(crate) fn new(initial: Bytes) -> Pump {
        Pump {
            pending: BytesMut::from(&initial[..]),
            scratch: vec![0u8; READ_BUF].into_boxed_slice(),
            first_byte: None,
            head_seen: BytesMut::new(),
            idle: None,
            deadline: None,
        }
    }

    /// Bytes read but not yet consumed by the parser.
    pub(crate) fn pending(&self) -> &[u8] {
        &self.pending
    }

    /// Takes the pending bytes (to forward them raw).
    pub(crate) fn take_pending(&mut self) -> Bytes {
        self.pending.split().freeze()
    }

    /// The bytes consumed for the last head read, parsed or not.
    pub(crate) fn head_seen(&self) -> &[u8] {
        &self.head_seen
    }

    /// Bounds each later read and write: at most `idle` per operation and
    /// nothing past `deadline` (`None` = no bound).
    pub(crate) fn set_limits(&mut self, idle: Option<Duration>, deadline: Option<Instant>) {
        self.idle = idle;
        self.deadline = deadline;
    }

    /// Starts timing: [`first_byte`](Pump::first_byte) becomes the time
    /// of the next read, or now when bytes are already pending.
    pub(crate) fn arm(&mut self) {
        self.first_byte = (!self.pending.is_empty()).then(Instant::now);
    }

    /// When the first byte since [`arm`](Pump::arm) arrived.
    pub(crate) fn first_byte(&self) -> Option<Instant> {
        self.first_byte
    }

    fn limit(&self) -> Limit {
        let idle = self.idle.map(|d| (Instant::now() + d, Clock::Idle));
        let total = self.deadline.map(|d| (d, Clock::Total));
        match (idle, total) {
            (Some(i), Some(t)) => Some(if t.0 <= i.0 { t } else { i }),
            (i, t) => i.or(t),
        }
    }

    /// Checks, without waiting, whether `from` has bytes or a close
    /// pending between messages. Bytes it reads are discarded: they must
    /// not be parsed as the next message.
    pub(crate) async fn probe<R>(&mut self, from: &mut R) -> Probe
    where
        R: AsyncRead + Unpin + ?Sized,
    {
        if !self.pending.is_empty() {
            return Probe::Stray;
        }
        let mut rb = ReadBuf::new(&mut self.scratch);
        std::future::poll_fn(|cx| {
            Poll::Ready(match Pin::new(&mut *from).poll_read(cx, &mut rb) {
                Poll::Pending => Probe::Idle,
                Poll::Ready(Ok(())) if rb.filled().is_empty() => Probe::Closed,
                Poll::Ready(Ok(())) => Probe::Stray,
                Poll::Ready(Err(_)) => Probe::Closed,
            })
        })
        .await
    }

    /// Reads once; returns the byte count (0 at EOF).
    async fn fill<R>(&mut self, from: &mut R) -> Result<usize, PumpFail>
    where
        R: AsyncRead + Unpin + ?Sized,
    {
        let limit = self.limit();
        let read = from.read(&mut self.scratch);
        let n = match limit {
            None => read.await,
            Some((until, clock)) => tokio::time::timeout_at(until.into(), read)
                .await
                .map_err(|_| PumpFail::Timeout(clock))?,
        }
        .map_err(PumpFail::Read)?;
        if n > 0 {
            self.first_byte.get_or_insert_with(Instant::now);
            self.pending.extend_from_slice(&self.scratch[..n]);
        }
        Ok(n)
    }

    /// Writes `bytes` to `to` under this pump's limits.
    pub(crate) async fn send<W>(&self, to: &mut W, bytes: &[u8]) -> Result<(), PumpFail>
    where
        W: AsyncWrite + Unpin + ?Sized,
    {
        write_limited(to, bytes, self.limit()).await
    }

    /// Reads one message head. The parser buffers head bytes, so nothing
    /// is forwarded: the caller writes the head (possibly rewritten).
    /// `None` when `from` closes cleanly before a message starts.
    pub(crate) async fn head<R>(
        &mut self,
        from: &mut R,
        parser: &mut Parser,
    ) -> Result<Option<RawMessage>, PumpFail>
    where
        R: AsyncRead + Unpin + ?Sized,
    {
        self.head_seen.clear();
        loop {
            if !self.pending.is_empty() {
                let step = parser.feed(&self.pending).map_err(PumpFail::Parse)?;
                self.head_seen
                    .extend_from_slice(&self.pending[..step.consumed]);
                self.pending.advance(step.consumed);
                match step.event {
                    Progress::Head(msg) => return Ok(Some(msg)),
                    Progress::NeedMore => {}
                    // A parser between messages only reports a head or
                    // asks for more; anything else is a broken message.
                    _ => return Err(PumpFail::Parse(ParseError::NoTerminator)),
                }
            }
            if self.fill(from).await? == 0 {
                return if parser.is_idle() && self.pending.is_empty() {
                    Ok(None)
                } else {
                    Err(PumpFail::Eof(ParseError::UnexpectedEof))
                };
            }
        }
    }

    /// Relays the body of the message whose head `parser` just returned:
    /// feeds pending bytes, writes exactly the consumed bytes to `to` (and
    /// `capture`) before reading more, re-presents what was not consumed,
    /// and calls `on_data` with each piece of decoded data.
    ///
    /// On a parse error the bytes consumed before it are forwarded first;
    /// the unparsable rest stays pending.
    pub(crate) async fn body<R, W>(
        &mut self,
        from: &mut R,
        parser: &mut Parser,
        to: &mut W,
        capture: &mut CaptureBuf,
        mut on_data: impl FnMut(&[u8]) -> Decision,
    ) -> Result<PumpEnd, PumpFail>
    where
        R: AsyncRead + Unpin + ?Sized,
        W: AsyncWrite + Unpin + ?Sized,
    {
        loop {
            let mut off = 0;
            let mut done = false;
            let mut failed = None;
            loop {
                let input = &self.pending[off..];
                let step = match parser.feed(input) {
                    Ok(step) => step,
                    Err(e) => {
                        failed = Some(e);
                        break;
                    }
                };
                if let Progress::BodyData { data, .. } = &step.event {
                    if on_data(&input[data.range()]) == Decision::Drop {
                        return Ok(PumpEnd::Dropped);
                    }
                }
                off += step.consumed;
                match step.event {
                    Progress::Done => {
                        done = true;
                        break;
                    }
                    Progress::NeedMore => break,
                    _ => {}
                }
            }
            if off > 0 {
                write_limited(to, &self.pending[..off], self.limit()).await?;
                capture.push(&self.pending[..off]);
                self.pending.advance(off);
            }
            if let Some(e) = failed {
                return Err(PumpFail::Parse(e));
            }
            if done {
                return Ok(PumpEnd::Done);
            }
            if self.fill(from).await? == 0 {
                return match parser.feed_eof() {
                    Ok(_) => Ok(PumpEnd::Eof),
                    Err(e) => Err(PumpFail::Eof(e)),
                };
            }
        }
    }
}

/// Writes and flushes `bytes`, failing with [`PumpFail::Timeout`] when
/// `limit` passes first.
async fn write_limited<W>(to: &mut W, bytes: &[u8], limit: Limit) -> Result<(), PumpFail>
where
    W: AsyncWrite + Unpin + ?Sized,
{
    let write = async {
        to.write_all(bytes).await?;
        to.flush().await
    };
    match limit {
        None => write.await,
        Some((until, clock)) => tokio::time::timeout_at(until.into(), write)
            .await
            .map_err(|_| PumpFail::Timeout(clock))?,
    }
    .map_err(PumpFail::Write)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Review focus: a slow reader holds back at most one read of the
    /// other side, never the whole body.
    #[test]
    fn read_buffer_is_64_kib() {
        assert_eq!(READ_BUF, 64 * 1024);
        let pump = Pump::new(Bytes::new());
        assert_eq!(pump.scratch.len(), 64 * 1024);
    }

    #[test]
    fn the_earlier_clock_wins() {
        let mut p = Pump::new(Bytes::new());
        assert!(p.limit().is_none());
        let soon = Instant::now() + Duration::from_millis(10);
        p.set_limits(Some(Duration::from_secs(60)), Some(soon));
        assert_eq!(p.limit(), Some((soon, Clock::Total)));
        p.set_limits(
            Some(Duration::from_millis(1)),
            Some(Instant::now() + Duration::from_secs(60)),
        );
        assert_eq!(p.limit().map(|l| l.1), Some(Clock::Idle));
    }
}
