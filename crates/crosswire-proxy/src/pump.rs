//! Moves one message from one connection to another through an h1 parser,
//! forwarding exactly the bytes the parser consumed.

use std::time::Instant;

use bytes::{Buf, Bytes, BytesMut};
use crosswire_client::Conn;
use crosswire_h1::{Parser, Progress, RawMessage};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::capture::Capture;
use crate::{Decision, ProxyError};

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

/// One direction of a connection: unconsumed bytes waiting to be
/// re-presented to the parser, and when the first byte of a read arrived.
///
/// Bounded buffering: a read happens only when the parser asks for more
/// (`NeedMore`), and the consumed bytes are written to the destination,
/// with the write awaited, before that read. At most [`READ_BUF`] bytes
/// plus one incomplete body token are held.
pub(crate) struct Pump {
    pending: BytesMut,
    scratch: Box<[u8]>,
    first_byte: Option<Instant>,
}

impl Pump {
    /// A pump whose first bytes are `initial` (already read elsewhere).
    pub(crate) fn new(initial: Bytes) -> Pump {
        Pump {
            pending: BytesMut::from(&initial[..]),
            scratch: vec![0u8; READ_BUF].into_boxed_slice(),
            first_byte: None,
        }
    }

    /// Bytes read but not yet consumed by the parser.
    pub(crate) fn pending(&self) -> &[u8] {
        &self.pending
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

    /// Reads once; returns the byte count (0 at EOF).
    async fn fill(&mut self, from: &mut dyn Conn) -> Result<usize, ProxyError> {
        let n = from.read(&mut self.scratch).await.map_err(ProxyError::Io)?;
        if n > 0 {
            self.first_byte.get_or_insert_with(Instant::now);
            self.pending.extend_from_slice(&self.scratch[..n]);
        }
        Ok(n)
    }

    /// Reads one message head. The parser buffers head bytes, so nothing
    /// is forwarded: the caller writes the head (possibly rewritten).
    /// `None` when `from` closes cleanly before a message starts.
    pub(crate) async fn head(
        &mut self,
        from: &mut dyn Conn,
        parser: &mut Parser,
    ) -> Result<Option<RawMessage>, ProxyError> {
        loop {
            if !self.pending.is_empty() {
                let step = parser.feed(&self.pending).map_err(parse_error)?;
                self.pending.advance(step.consumed);
                match step.event {
                    Progress::Head(msg) => return Ok(Some(msg)),
                    Progress::NeedMore => {}
                    other => {
                        return Err(ProxyError::Protocol(format!(
                            "unexpected {other:?} before a head"
                        )))
                    }
                }
            }
            if self.fill(from).await? == 0 {
                return if parser.is_idle() && self.pending.is_empty() {
                    Ok(None)
                } else {
                    Err(ProxyError::Protocol("connection closed mid-head".into()))
                };
            }
        }
    }

    /// Relays the body of the message whose head `parser` just returned:
    /// feeds pending bytes, writes exactly the consumed bytes to `to` (and
    /// `capture`) before reading more, re-presents what was not consumed,
    /// and calls `on_data` with each piece of decoded data.
    pub(crate) async fn body(
        &mut self,
        from: &mut dyn Conn,
        parser: &mut Parser,
        to: &mut dyn Conn,
        capture: &mut Capture,
        mut on_data: impl FnMut(&[u8]) -> Decision,
    ) -> Result<PumpEnd, ProxyError> {
        loop {
            let mut off = 0;
            let mut done = false;
            loop {
                let input = &self.pending[off..];
                let step = parser.feed(input).map_err(parse_error)?;
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
                to.write_all(&self.pending[..off])
                    .await
                    .map_err(ProxyError::Io)?;
                to.flush().await.map_err(ProxyError::Io)?;
                capture.push(&self.pending[..off]);
                self.pending.advance(off);
            }
            if done {
                return Ok(PumpEnd::Done);
            }
            if self.fill(from).await? == 0 {
                parser
                    .feed_eof()
                    .map_err(|_| ProxyError::Protocol("connection closed mid-body".into()))?;
                return Ok(PumpEnd::Eof);
            }
        }
    }
}

fn parse_error(e: crosswire_h1::ParseError) -> ProxyError {
    ProxyError::Protocol(format!("parse error: {e:?}"))
}
