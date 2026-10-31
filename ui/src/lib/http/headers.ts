// Header-line parser for the Headers tab. Never decodes: bytes are read as
// Latin-1 (1:1). Lines split on CRLF or LF; a lone CR stays in the text.
// An obs-fold continuation (leading SP/HT) joins the previous value.
//
// NOTE: minimal local version so the Repeater builds on its own; Task 2 owns
// this module and its version replaces this one on merge.
import { bytesToLatin1 } from '../editor/encoding';

export interface HeaderLine {
  name: string;
  value: string;
}

export interface ParsedHead {
  start: string;
  headers: HeaderLine[];
  malformed: boolean;
}

export function parseHeadLines(bytes: Uint8Array): ParsedHead {
  const lines = bytesToLatin1(bytes).split(/\r?\n/);
  const start = lines[0] ?? '';
  const headers: HeaderLine[] = [];
  let malformed = false;
  for (let i = 1; i < lines.length; i++) {
    const line = lines[i];
    if (line === '') break;
    if ((line[0] === ' ' || line[0] === '\t') && headers.length > 0) {
      const prev = headers[headers.length - 1];
      prev.value = `${prev.value} ${line.trim()}`;
      continue;
    }
    const colon = line.indexOf(':');
    if (colon <= 0) {
      malformed = true;
      continue;
    }
    headers.push({ name: line.slice(0, colon), value: line.slice(colon + 1).trim() });
  }
  return { start, headers, malformed };
}
