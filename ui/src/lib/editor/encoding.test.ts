import { describe, it, expect } from 'vitest';
import { bytesToLatin1, latin1ToBytes, encodePastedText } from './encoding';

describe('Latin-1 byte mapping', () => {
  it('round-trips every single byte 0x00-0xFF exactly', () => {
    for (let b = 0; b < 256; b++) {
      const input = new Uint8Array([b]);
      const back = latin1ToBytes(bytesToLatin1(input));
      expect(Array.from(back), `byte 0x${b.toString(16)}`).toEqual([b]);
    }
  });

  it('maps byte b to code point U+00b (forward direction)', () => {
    for (let b = 0; b < 256; b++) {
      const s = bytesToLatin1(new Uint8Array([b]));
      expect(s.length).toBe(1);
      expect(s.charCodeAt(0)).toBe(b);
    }
  });

  it('round-trips a full-range 256-byte buffer', () => {
    const full = new Uint8Array(256);
    for (let b = 0; b < 256; b++) full[b] = b;
    const back = latin1ToBytes(bytesToLatin1(full));
    expect(Array.from(back)).toEqual(Array.from(full));
  });

  it('round-trips a large buffer without stack overflow', () => {
    const big = new Uint8Array(500_000);
    for (let i = 0; i < big.length; i++) big[i] = i & 0xff;
    const back = latin1ToBytes(bytesToLatin1(big));
    expect(back.length).toBe(big.length);
    expect(back[0]).toBe(0);
    expect(back[499_999]).toBe(499_999 & 0xff);
  });

  it('round-trips 0x00 (NUL) exactly', () => {
    const back = latin1ToBytes(bytesToLatin1(new Uint8Array([0x00])));
    expect(Array.from(back)).toEqual([0x00]);
  });

  it('round-trips buffers with lone \\r, lone \\n, and \\r\\n', () => {
    const cases: number[][] = [
      [0x61, 0x0d, 0x62], // a \r b
      [0x61, 0x0a, 0x62], // a \n b
      [0x61, 0x0d, 0x0a, 0x62], // a \r\n b
      [0x0d, 0x0a, 0x0d, 0x0a], // \r\n \r\n
      [0x0d], // lone \r
      [0x0a], // lone \n
    ];
    for (const c of cases) {
      const input = new Uint8Array(c);
      const back = latin1ToBytes(bytesToLatin1(input));
      expect(Array.from(back), JSON.stringify(c)).toEqual(c);
    }
  });
});

describe('encodePastedText', () => {
  it('flags non-Latin-1 text and UTF-8 encodes it', () => {
    const { bytes, nonLatin1 } = encodePastedText('€'); // EURO SIGN
    expect(nonLatin1).toBe(true);
    expect(Array.from(bytes)).toEqual([0xe2, 0x82, 0xac]);
  });

  it('does not flag pure ASCII and UTF-8 encodes it', () => {
    const { bytes, nonLatin1 } = encodePastedText('abc');
    expect(nonLatin1).toBe(false);
    expect(Array.from(bytes)).toEqual([0x61, 0x62, 0x63]);
  });

  it('does not flag text whose chars are all within Latin-1 (<= 0xFF)', () => {
    const { bytes, nonLatin1 } = encodePastedText('ÿé'); // ÿ é
    expect(nonLatin1).toBe(false);
    // Latin-1 chars above 0x7F are multi-byte in UTF-8.
    expect(Array.from(bytes)).toEqual([0xc3, 0xbf, 0xc3, 0xa9]);
  });
});
