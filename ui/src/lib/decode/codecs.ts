// Pure bytes-in/bytes-out codecs for the decoder pane, plus a non-verifying
// JWT view. Everything works on raw bytes so arbitrary (non-UTF-8) payloads
// round-trip; only the JWT view's header/payload are text.
//
// `apply` never throws: malformed input is kept or skipped and reported via a
// warning, because the pane re-runs on every keystroke over half-typed input.

export type Codec = 'url' | 'base64' | 'hex' | 'html';
export const CODECS: readonly Codec[] = ['url', 'base64', 'hex', 'html'];
export type Direction = 'encode' | 'decode';

export interface CodecResult {
  bytes: Uint8Array;
  warning: string | null;
}

const ascii = (s: string) => Uint8Array.from(s, (c) => c.charCodeAt(0));
const isWs = (b: number) => b === 0x20 || b === 0x09 || b === 0x0a || b === 0x0d || b === 0x0b || b === 0x0c;

function hexVal(b: number): number {
  if (b >= 0x30 && b <= 0x39) return b - 0x30;
  if (b >= 0x41 && b <= 0x46) return b - 0x41 + 10;
  if (b >= 0x61 && b <= 0x66) return b - 0x61 + 10;
  return -1;
}

// ---- url ----------------------------------------------------------------

const URL_SAFE = /[A-Za-z0-9\-._~]/;
const HEX_UP = '0123456789ABCDEF';

function urlEncode(input: Uint8Array): CodecResult {
  const out: number[] = [];
  for (const b of input) {
    if (URL_SAFE.test(String.fromCharCode(b))) out.push(b);
    else out.push(0x25, HEX_UP.charCodeAt(b >> 4), HEX_UP.charCodeAt(b & 15));
  }
  return { bytes: Uint8Array.from(out), warning: null };
}

function urlDecode(input: Uint8Array): CodecResult {
  const out: number[] = [];
  let bad = 0;
  for (let i = 0; i < input.length; i++) {
    const b = input[i];
    if (b === 0x25) {
      const h = i + 2 < input.length ? hexVal(input[i + 1]) : -1;
      const l = i + 2 < input.length ? hexVal(input[i + 2]) : -1;
      if (h >= 0 && l >= 0) {
        out.push((h << 4) | l);
        i += 2;
        continue;
      }
      bad++;
    }
    out.push(b);
  }
  return {
    bytes: Uint8Array.from(out),
    warning: bad ? `url: ${bad} malformed % sequences kept literally` : null,
  };
}

// ---- base64 -------------------------------------------------------------

const B64 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';

function b64Val(b: number): number {
  if (b === 0x2d) return 62; // '-' url-safe
  if (b === 0x5f) return 63; // '_' url-safe
  return b < 128 ? B64.indexOf(String.fromCharCode(b)) : -1;
}

function base64Encode(input: Uint8Array): CodecResult {
  let s = '';
  for (let i = 0; i < input.length; i += 3) {
    const n = (input[i] << 16) | ((input[i + 1] ?? 0) << 8) | (input[i + 2] ?? 0);
    s += B64[(n >> 18) & 63] + B64[(n >> 12) & 63];
    s += i + 1 < input.length ? B64[(n >> 6) & 63] : '=';
    s += i + 2 < input.length ? B64[n & 63] : '=';
  }
  return { bytes: ascii(s), warning: null };
}

function base64Decode(input: Uint8Array): CodecResult {
  const out: number[] = [];
  let acc = 0;
  let bits = 0;
  let bad = 0;
  for (const b of input) {
    if (isWs(b) || b === 0x3d) continue; // whitespace and '=' padding are silent
    const v = b64Val(b);
    if (v < 0) {
      bad++;
      continue;
    }
    acc = ((acc << 6) | v) & 0xffffff;
    bits += 6;
    if (bits >= 8) {
      bits -= 8;
      out.push((acc >> bits) & 0xff);
    }
  }
  return {
    bytes: Uint8Array.from(out),
    warning: bad ? `base64: ${bad} invalid characters skipped` : null,
  };
}

// ---- hex ----------------------------------------------------------------

function hexEncode(input: Uint8Array): CodecResult {
  const lower = '0123456789abcdef';
  const out = new Uint8Array(input.length * 2);
  input.forEach((b, i) => {
    out[2 * i] = lower.charCodeAt(b >> 4);
    out[2 * i + 1] = lower.charCodeAt(b & 15);
  });
  return { bytes: out, warning: null };
}

function hexDecode(input: Uint8Array): CodecResult {
  let start = 0;
  while (start < input.length && isWs(input[start])) start++;
  if (input[start] === 0x30 && (input[start + 1] | 0x20) === 0x78) start += 2; // 0x / 0X
  const nibbles: number[] = [];
  let bad = 0;
  for (let i = start; i < input.length; i++) {
    if (isWs(input[i])) continue;
    const v = hexVal(input[i]);
    if (v < 0) bad++;
    else nibbles.push(v);
  }
  const odd = nibbles.length % 2 === 1;
  if (odd) nibbles.pop();
  const out = new Uint8Array(nibbles.length / 2);
  for (let i = 0; i < out.length; i++) out[i] = (nibbles[2 * i] << 4) | nibbles[2 * i + 1];
  const warnings: string[] = [];
  if (bad) warnings.push(`hex: ${bad} invalid characters skipped`);
  if (odd) warnings.push('hex: odd length, last nibble dropped');
  return { bytes: out, warning: warnings.length ? warnings.join('; ') : null };
}

// ---- html entities ------------------------------------------------------

const NAMED: Record<string, number> = { amp: 0x26, lt: 0x3c, gt: 0x3e, quot: 0x22, apos: 0x27, nbsp: 0xa0 };
const ENTITY = /^&(#[0-9]+|#[xX][0-9a-fA-F]+|[A-Za-z][A-Za-z0-9]*);/;
const HTML_ESC: Record<number, string> = {
  0x26: '&amp;',
  0x3c: '&lt;',
  0x3e: '&gt;',
  0x22: '&quot;',
  0x27: '&#39;',
};

function htmlEncode(input: Uint8Array): CodecResult {
  const out: number[] = [];
  for (const b of input) {
    const esc = HTML_ESC[b];
    if (esc) for (const c of esc) out.push(c.charCodeAt(0));
    else out.push(b);
  }
  return { bytes: Uint8Array.from(out), warning: null };
}

function entityCodePoint(body: string): number | null {
  let cp: number | undefined;
  if (body[0] === '#') {
    cp = body[1] === 'x' || body[1] === 'X' ? parseInt(body.slice(2), 16) : parseInt(body.slice(1), 10);
  } else {
    cp = NAMED[body];
  }
  if (cp === undefined || !Number.isFinite(cp)) return null;
  // Reject NUL, surrogates and out-of-range values rather than emit U+FFFD.
  if (cp < 1 || cp > 0x10ffff || (cp >= 0xd800 && cp <= 0xdfff)) return null;
  return cp;
}

function htmlDecode(input: Uint8Array): CodecResult {
  const out: number[] = [];
  let unknown = 0;
  for (let i = 0; i < input.length; i++) {
    if (input[i] !== 0x26) {
      out.push(input[i]);
      continue;
    }
    // Entities are ASCII; look at a bounded window so this stays linear.
    const window = String.fromCharCode(...input.subarray(i, i + 12));
    const m = ENTITY.exec(window);
    if (!m) {
      out.push(0x26); // lone '&' is ordinary text, not an entity
      continue;
    }
    const cp = entityCodePoint(m[1]);
    if (cp === null) {
      unknown++;
      out.push(0x26);
      continue;
    }
    out.push(...new TextEncoder().encode(String.fromCodePoint(cp)));
    i += m[0].length - 1;
  }
  return {
    bytes: Uint8Array.from(out),
    warning: unknown ? `html: ${unknown} unknown entities kept literally` : null,
  };
}

// ---- public API ---------------------------------------------------------

const TABLE: Record<Codec, Record<Direction, (b: Uint8Array) => CodecResult>> = {
  url: { encode: urlEncode, decode: urlDecode },
  base64: { encode: base64Encode, decode: base64Decode },
  hex: { encode: hexEncode, decode: hexDecode },
  html: { encode: htmlEncode, decode: htmlDecode },
};

export function apply(codec: Codec, dir: Direction, input: Uint8Array): CodecResult {
  try {
    return TABLE[codec][dir](input);
  } catch {
    // Defence in depth: the contract is "never throws".
    return { bytes: new Uint8Array(0), warning: `${codec}: could not ${dir} input` };
  }
}

// ---- JWT view -----------------------------------------------------------

export interface JwtView {
  header: string;
  payload: string;
  signature: string;
  warnings: string[];
}

function decodeSegment(seg: string): string {
  const bytes = base64Decode(new TextEncoder().encode(seg)).bytes;
  const text = new TextDecoder('utf-8').decode(bytes);
  try {
    return JSON.stringify(JSON.parse(text), null, 2);
  } catch {
    return text;
  }
}

function headerAlg(header: string): unknown {
  try {
    const h: unknown = JSON.parse(header);
    return h !== null && typeof h === 'object' ? (h as Record<string, unknown>).alg : undefined;
  } catch {
    return undefined;
  }
}

/** Decode (never verify) a JWT; null unless exactly three `.`-separated segments. */
export function jwtView(input: Uint8Array): JwtView | null {
  const parts = new TextDecoder('utf-8').decode(input).trim().split('.');
  if (parts.length !== 3) return null;
  const header = decodeSegment(parts[0]);
  const payload = decodeSegment(parts[1]);
  const warnings = ['signature not verified'];
  const alg = headerAlg(header);
  if (alg === 'none') warnings.push('alg is none');
  else if (alg === undefined) warnings.push('alg missing');
  return { header, payload, signature: parts[2], warnings };
}
