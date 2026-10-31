// Header-line view of a recorded HTTP/1 head, for the detail Headers tab.
//
// The bytes are NOT decoded: each byte maps 1:1 to a code point through the
// Latin-1 mapping (bytesToLatin1), so 0x80-0xFF show as their Latin-1 glyphs
// and nothing is guessed. Lines end at CRLF, a bare LF or a lone CR, so no CR
// or LF ever reaches an output string. Parsing stops at the first empty line
// (the end of the head). Never throws: anything it cannot classify is listed
// in `malformed` rather than dropped.

import { bytesToLatin1 } from '../editor/encoding';

export interface HeaderLine {
  name: string;
  value: string;
}

export interface HeadLines {
  /** The request or status line ('' when the head is empty). */
  start: string;
  headers: HeaderLine[];
  /** Lines that are neither a header nor an obs-fold continuation. */
  malformed: string[];
}

const OWS = /^[ \t]+|[ \t]+$/g;

export function parseHeadLines(bytes: Uint8Array): HeadLines {
  const out: HeadLines = { start: '', headers: [], malformed: [] };
  if (bytes.length === 0) return out;
  const lines = bytesToLatin1(bytes).split(/\r\n|\n|\r/);
  out.start = lines[0] ?? '';
  for (let i = 1; i < lines.length; i++) {
    const line = lines[i];
    if (line === '') break;
    const first = line.charCodeAt(0);
    if (first === 0x20 || first === 0x09) {
      // obs-fold (RFC 9112 §5.2): continues the previous field value.
      const prev = out.headers[out.headers.length - 1];
      if (prev) {
        const more = line.replace(OWS, '');
        prev.value = prev.value === '' ? more : more === '' ? prev.value : `${prev.value} ${more}`;
      } else {
        out.malformed.push(line);
      }
      continue;
    }
    const colon = line.indexOf(':');
    if (colon < 0) {
      out.malformed.push(line);
      continue;
    }
    out.headers.push({ name: line.slice(0, colon), value: line.slice(colon + 1).replace(OWS, '') });
  }
  return out;
}

/** Make CR and LF visible (U+240D / U+240A) in a value that did not come from a split head. */
export function visibleLineBreaks(s: string): string {
  return s.replace(/\r/g, '␍').replace(/\n/g, '␊');
}
