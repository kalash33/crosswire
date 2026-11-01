// Pretty renderings of recorded bytes (U2 D1). Pure text in, text out: the
// results are shown as text only (§11.1) and never leave the UI.

import { latin1ToBytes } from '../editor/encoding';

const LIMIT = 2 * 1024 * 1024; // = detect.PRETTY_LIMIT (kept local: no import cycle)

/** Non-fatal UTF-8 decode (invalid sequences become U+FFFD; a BOM is dropped). */
export function decodeUtf8(bytes: Uint8Array): string {
  return new TextDecoder('utf-8').decode(bytes);
}

/** True when `bytes` is strict UTF-8 (a lossy decode would add U+FFFD). */
export function isValidUtf8(bytes: Uint8Array): boolean {
  try {
    new TextDecoder('utf-8', { fatal: true }).decode(bytes);
    return true;
  } catch {
    return false;
  }
}

/**
 * Re-indent a JSON body with two spaces, the layout of
 * `JSON.stringify(v, null, 2)`. The body is validated with `JSON.parse`, but
 * the output is built from the RECORDED text, not the parsed value, so what a
 * tester sees is what was sent: number literals keep every digit, string
 * escapes stay as written, key order and duplicate keys are kept. Returns null
 * for invalid JSON or a body over 2 MiB.
 */
export function prettyJson(bytes: Uint8Array): string | null {
  if (bytes.length === 0 || bytes.length > LIMIT) return null;
  const text = decodeUtf8(bytes);
  try {
    JSON.parse(text);
  } catch {
    return null;
  }
  return reindent(text);
}

function reindent(text: string): string {
  const out: string[] = [];
  let depth = 0;
  const nl = () => '\n' + '  '.repeat(depth);
  let i = 0;
  const n = text.length;
  while (i < n) {
    const c = text[i];
    if (c === '"') {
      // Copy the string literal verbatim (valid JSON: it is terminated).
      let j = i + 1;
      while (j < n && text[j] !== '"') j += text[j] === '\\' ? 2 : 1;
      out.push(text.slice(i, j + 1));
      i = j + 1;
      continue;
    }
    if (c === '{' || c === '[') {
      // Empty containers stay on one line, as JSON.stringify writes them.
      let j = i + 1;
      while (j < n && isBlank(text.charCodeAt(j))) j++;
      const close = c === '{' ? '}' : ']';
      if (text[j] === close) {
        out.push(c + close);
        i = j + 1;
        continue;
      }
      depth++;
      out.push(c + nl());
    } else if (c === '}' || c === ']') {
      depth--;
      out.push(nl() + c);
    } else if (c === ',') {
      out.push(',' + nl());
    } else if (c === ':') {
      out.push(': ');
    } else if (!isBlank(text.charCodeAt(i))) {
      // A number or literal: copy the run as written.
      let j = i + 1;
      while (j < n && !isBlank(text.charCodeAt(j)) && !',:]}'.includes(text[j])) j++;
      out.push(text.slice(i, j));
      i = j;
      continue;
    }
    i++;
  }
  return out.join('');
}

function isBlank(code: number): boolean {
  return code === 0x20 || code === 0x09 || code === 0x0a || code === 0x0d || code === 0xfeff;
}

export interface KeyValue {
  key: string;
  value: string;
}

/**
 * `application/x-www-form-urlencoded` pairs: split on `&` (empty segments
 * skipped) and the first `=`, `+` read as a space, `%XX` decoded to bytes and
 * the result read as UTF-8. A malformed `%` sequence is kept literally.
 */
export function parseForm(bytes: Uint8Array, max = Infinity): KeyValue[] {
  const out: KeyValue[] = [];
  let start = 0;
  for (let i = 0; i <= bytes.length && out.length < max; i++) {
    if (i < bytes.length && bytes[i] !== 0x26) continue;
    if (i > start) {
      const seg = bytes.subarray(start, i);
      const eq = seg.indexOf(0x3d);
      out.push(
        eq < 0
          ? { key: formDecode(seg), value: '' }
          : { key: formDecode(seg.subarray(0, eq)), value: formDecode(seg.subarray(eq + 1)) },
      );
    }
    start = i + 1;
  }
  return out;
}

/** The number of pairs `parseForm` would return, without decoding any. */
export function countPairs(bytes: Uint8Array): number {
  let n = 0;
  let start = 0;
  for (let i = 0; i <= bytes.length; i++) {
    if (i < bytes.length && bytes[i] !== 0x26) continue;
    if (i > start) n++;
    start = i + 1;
  }
  return n;
}

/** The query of a request target (after `?`, before `#`) as key/value pairs. */
export function parseQuery(target: string): KeyValue[] {
  const q = target.indexOf('?');
  if (q < 0) return [];
  const hash = target.indexOf('#', q);
  return parseForm(latin1ToBytes(target.slice(q + 1, hash < 0 ? undefined : hash)));
}

function hexVal(b: number): number {
  if (b >= 0x30 && b <= 0x39) return b - 0x30;
  if (b >= 0x41 && b <= 0x46) return b - 0x37;
  if (b >= 0x61 && b <= 0x66) return b - 0x57;
  return -1;
}

function formDecode(seg: Uint8Array): string {
  const out = new Uint8Array(seg.length);
  let o = 0;
  for (let i = 0; i < seg.length; i++) {
    const b = seg[i];
    if (b === 0x2b) {
      out[o++] = 0x20;
    } else if (b === 0x25 && i + 2 < seg.length && hexVal(seg[i + 1]) >= 0 && hexVal(seg[i + 2]) >= 0) {
      out[o++] = hexVal(seg[i + 1]) * 16 + hexVal(seg[i + 2]);
      i += 2;
    } else {
      out[o++] = b;
    }
  }
  return decodeUtf8(out.subarray(0, o));
}
