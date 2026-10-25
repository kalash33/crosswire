// Byte-exact Latin-1 (ISO-8859-1) mapping between raw bytes and strings.
//
// The product requirement (§7.2) is a 1:1 Latin-1 mapping where every byte
// 0x00-0xFF round-trips: byte `b` <-> code point U+00b. This module is the
// single place that mapping lives; ByteEditor and HexPane build on it and must
// NEVER route bytes through any other (lossy) string operation.
//
// Decoder choice — verified, not assumed:
//   `TextDecoder('iso-8859-1')` / `TextDecoder('latin1')` do NOT work here.
//   The WHATWG Encoding Standard aliases both labels to windows-1252, which
//   remaps 0x80-0x9F to assorted printable code points (e.g. 0x80 -> U+20AC).
//   That silently corrupts 27 of the 256 byte values. We verified this against
//   the actual runtime before choosing the implementation, so we build the
//   string from char codes directly, which is exact for all 256 values.
//
//   `String.fromCharCode(...bytes)` (spread) overflows the call stack on large
//   buffers, so we apply it over bounded chunks instead.

const CHUNK = 0x8000; // 32768 — comfortably under argument-count limits.

/**
 * Map raw bytes to a string via the 1:1 Latin-1 mapping (byte b -> U+00b).
 * Exact for every byte 0x00-0xFF; safe for arbitrarily large buffers.
 */
export function bytesToLatin1(bytes: Uint8Array): string {
  let out = '';
  for (let i = 0; i < bytes.length; i += CHUNK) {
    const chunk = bytes.subarray(i, i + CHUNK);
    out += String.fromCharCode.apply(null, chunk as unknown as number[]);
  }
  return out;
}

/**
 * Map a Latin-1 string back to raw bytes. The exact inverse of
 * {@link bytesToLatin1} over all 256 byte values. Characters above 0xFF are
 * masked with `& 0xFF`; such input is not produced by `bytesToLatin1` and only
 * arises from external string handling (paste is routed through
 * {@link encodePastedText} instead, which preserves non-Latin-1 text as UTF-8).
 */
export function latin1ToBytes(s: string): Uint8Array {
  const out = new Uint8Array(s.length);
  for (let i = 0; i < s.length; i++) {
    out[i] = s.charCodeAt(i) & 0xff;
  }
  return out;
}

const UTF8_ENCODER = new TextEncoder();

/**
 * Encode text pasted into the editor. Text is UTF-8 encoded (the wire-safe
 * default), and `nonLatin1` is true iff any character falls outside Latin-1
 * (code point > 0xFF) — the signal the UI surfaces as a chip so the user knows
 * their paste was re-encoded rather than mapped 1:1.
 */
export function encodePastedText(s: string): { bytes: Uint8Array; nonLatin1: boolean } {
  let nonLatin1 = false;
  for (let i = 0; i < s.length; i++) {
    if (s.charCodeAt(i) > 0xff) {
      nonLatin1 = true;
      break;
    }
  }
  return { bytes: UTF8_ENCODER.encode(s), nonLatin1 };
}
