//! Standard-alphabet base64 (RFC 4648 section 4) with padding.
//!
//! Written here so the schema crate needs no new dependency. Only the
//! `std`-feature serialisation of [`Bytes`](bytes::Bytes) uses it.

use alloc::string::String;
use alloc::vec::Vec;

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Encodes `input` as padded base64.
pub(crate) fn encode(input: &[u8]) -> String {
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn value(c: u8) -> Option<u32> {
    match c {
        b'A'..=b'Z' => Some(u32::from(c - b'A')),
        b'a'..=b'z' => Some(u32::from(c - b'a') + 26),
        b'0'..=b'9' => Some(u32::from(c - b'0') + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

/// Decodes padded base64. Returns `None` on any character outside the
/// alphabet, bad padding, a length that is not a multiple of four, or
/// non-zero trailing bits.
pub(crate) fn decode(input: &str) -> Option<Vec<u8>> {
    let s = input.as_bytes();
    if s.len() % 4 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let quads = s.len() / 4;
    for (qi, q) in s.chunks(4).enumerate() {
        let last = qi + 1 == quads;
        let pad = q.iter().rev().take_while(|&&c| c == b'=').count();
        if pad > 2 || (pad > 0 && !last) {
            return None;
        }
        let mut n = 0u32;
        for (i, &c) in q.iter().enumerate() {
            let v = if i >= 4 - pad { 0 } else { value(c)? };
            n = (n << 6) | v;
        }
        let bytes = n.to_be_bytes();
        let take = 3 - pad;
        // Canonical form: unused low bits must be zero.
        if bytes[1 + take..4].iter().any(|&b| b != 0) {
            return None;
        }
        out.extend_from_slice(&bytes[1..1 + take]);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc4648_vectors() {
        let v: [(&[u8], &str); 7] = [
            (b"", ""),
            (b"f", "Zg=="),
            (b"fo", "Zm8="),
            (b"foo", "Zm9v"),
            (b"foob", "Zm9vYg=="),
            (b"fooba", "Zm9vYmE="),
            (b"foobar", "Zm9vYmFy"),
        ];
        for (raw, enc) in v {
            assert_eq!(encode(raw), enc);
            assert_eq!(decode(enc).as_deref(), Some(raw));
        }
    }

    #[test]
    fn all_byte_values_roundtrip() {
        let all: Vec<u8> = (0..=255).collect();
        assert_eq!(decode(&encode(&all)), Some(all));
        assert_eq!(encode(&[0, 255, b'h', b'i']), "AP9oaQ==");
    }

    #[test]
    fn rejects_malformed() {
        for bad in [
            "Zg=", "Zg", "Z===", "=Zg=", "Zg==Zm9v", "Zm9!", "Zh==", "Zm9 ", "Zm8*",
        ] {
            assert_eq!(decode(bad), None, "{bad}");
        }
    }
}
