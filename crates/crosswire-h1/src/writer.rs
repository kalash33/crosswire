//! Byte-exact output: the inverse of parsing.
use alloc::vec::Vec;
use bytes::{BufMut, BytesMut};

use crate::message::RawMessage;

/// Appends the message to `out` exactly as it was received: the frozen head
/// bytes, then the inline body if one is attached. Nothing is normalised,
/// added or removed.
///
/// The inline body is written as is, so it must be the raw framed body bytes
/// (chunk-size lines, extensions and trailers included), as documented on
/// [`RawMessage::with_inline_body`]; then the output equals the input
/// message byte for byte. A decoded payload attached to a chunked message
/// would not reproduce it.
pub fn write_exact(msg: &RawMessage, out: &mut BytesMut) {
    out.extend_from_slice(msg.bytes());
    if let Some(body) = msg.body() {
        out.extend_from_slice(body);
    }
}

/// Writes a chunked message body, chunk by chunk.
///
/// Framing is whatever the caller asks for; the encoder only adds the chunk
/// syntax. Trailers given to [`BodyEncoder::trailers`] are held and written by
/// [`BodyEncoder::finish`].
#[derive(Debug, Default, Clone)]
pub struct BodyEncoder {
    trailers: Option<Vec<(Vec<u8>, Vec<u8>)>>,
}

impl BodyEncoder {
    /// Creates an encoder for a chunked body.
    #[must_use]
    pub fn chunked() -> Self {
        BodyEncoder { trailers: None }
    }

    /// Appends one chunk: `{len as lowercase hex}[;ext]\r\n{data}\r\n`.
    ///
    /// An empty `data` writes nothing, because a zero-length chunk would
    /// terminate the body; use [`BodyEncoder::finish`] for that.
    pub fn chunk(&mut self, data: &[u8], ext: Option<&[u8]>, out: &mut BytesMut) {
        if data.is_empty() {
            return;
        }
        put_hex(data.len(), out);
        if let Some(ext) = ext {
            out.put_u8(b';');
            out.extend_from_slice(ext);
        }
        out.extend_from_slice(b"\r\n");
        out.extend_from_slice(data);
        out.extend_from_slice(b"\r\n");
    }

    /// Records trailer fields to be written by [`BodyEncoder::finish`],
    /// replacing any recorded earlier. Nothing is written to `out` yet.
    pub fn trailers(&mut self, t: &[(&[u8], &[u8])], _out: &mut BytesMut) {
        self.trailers = Some(t.iter().map(|(n, v)| (n.to_vec(), v.to_vec())).collect());
    }

    /// Writes the terminating `0\r\n`, the recorded trailers as
    /// `name: value\r\n`, and the final blank line.
    pub fn finish(&mut self, out: &mut BytesMut) {
        out.extend_from_slice(b"0\r\n");
        for (name, value) in self.trailers.take().unwrap_or_default() {
            out.extend_from_slice(&name);
            out.extend_from_slice(b": ");
            out.extend_from_slice(&value);
            out.extend_from_slice(b"\r\n");
        }
        out.extend_from_slice(b"\r\n");
    }
}

fn put_hex(mut n: usize, out: &mut BytesMut) {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut buf = [0u8; 16];
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = DIGITS[n & 0xf];
        n >>= 4;
        if n == 0 {
            break;
        }
    }
    out.extend_from_slice(&buf[i..]);
}
