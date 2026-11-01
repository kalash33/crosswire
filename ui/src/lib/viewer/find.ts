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

/** Find stops counting here; the bar then shows "10000+". */
export const FIND_LIMIT = 10_000;

export interface FindOptions {
  caseSensitive: boolean;
  /** `foldCase(text)`, when the caller caches it across keystrokes. */
  folded?: string;
}

// Lowercase one UTF-16 unit at a time so offsets never move (a whole-string
// toLowerCase can change the length, e.g. U+0130).
export function foldCase(s: string): string {
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

/**
 * Start offsets of the non-overlapping matches of `needle` in `text`, at most
 * FIND_LIMIT + 1 of them (one past the limit tells the caller there are more),
 * so a one-character needle over a megabyte never builds a huge mark set.
 */
export function findAll(text: string, needle: string, opts: FindOptions): number[] {
  if (needle === '') return [];
  const hay = opts.caseSensitive ? text : (opts.folded ?? foldCase(text));
  const pin = opts.caseSensitive ? needle : foldCase(needle);
  const out: number[] = [];
  let at = hay.indexOf(pin);
  while (at >= 0 && out.length <= FIND_LIMIT) {
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
