// Find-in-message for the viewer (U2 D1): a plain incremental search over the
// text the editor shows (the Latin-1 view of the bytes, or the Pretty text).
// No CodeMirror search package: matches come back as offsets and the editor
// marks them with a decoration.

/** Find matches an editor marks: start offsets, one length, the current index (-1: none). */
export interface EditorMatches {
  positions: readonly number[];
  length: number;
  current: number;
}

export interface FindOptions {
  caseSensitive: boolean;
}

// Lowercase one UTF-16 unit at a time so offsets never move (a whole-string
// toLowerCase can change the length, e.g. U+0130).
function fold(s: string): string {
  let out = '';
  for (let i = 0; i < s.length; i++) {
    const c = s.charCodeAt(i);
    if ((c >= 0x41 && c <= 0x5a) || (c >= 0xc0 && c <= 0xde && c !== 0xd7)) {
      out += String.fromCharCode(c + 0x20);
    } else if (c <= 0xff) {
      out += s[i];
    } else {
      const l = s[i].toLowerCase();
      out += l.length === 1 ? l : s[i];
    }
  }
  return out;
}

/** Start offsets of every non-overlapping match of `needle` in `text`. */
export function findAll(text: string, needle: string, opts: FindOptions): number[] {
  if (needle === '') return [];
  const hay = opts.caseSensitive ? text : fold(text);
  const pin = opts.caseSensitive ? needle : fold(needle);
  const out: number[] = [];
  let at = hay.indexOf(pin);
  while (at >= 0) {
    out.push(at);
    at = hay.indexOf(pin, at + pin.length);
  }
  return out;
}

/** The match after `current`, wrapping to the first (-1 when there are none). */
export function nextMatch(count: number, current: number): number {
  if (count <= 0) return -1;
  return current < 0 ? 0 : (current + 1) % count;
}

/** The match before `current`, wrapping to the last (-1 when there are none). */
export function prevMatch(count: number, current: number): number {
  if (count <= 0) return -1;
  return current <= 0 ? count - 1 : current - 1;
}
