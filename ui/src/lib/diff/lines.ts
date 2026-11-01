// A dependency-free line diff (Myers' O((N+M)D) shortest edit script, which is
// a longest common subsequence of lines, in linear space). Used by the
// repeater's "Compare with previous" view over two responses' Raw text.
//
// Speed-ups that keep the result minimal:
//  - lines are interned to integers, so comparisons are integer compares;
//  - the common prefix and suffix are matched up front;
//  - a line that never occurs on the other side cannot be in any common
//    subsequence, so it is dropped before the search and re-inserted as a
//    plain delete/add afterwards (the GNU diff "discard" step).
//
// The output is data for a text-only view: lines are never interpreted.

export type DiffKind = 'same' | 'add' | 'del';

export interface DiffLine {
  kind: DiffKind;
  text: string;
}

/** Each side is bounded; beyond it the caller shows "too large to diff". */
export const DIFF_MAX_LINES = 20000;

export function tooLargeToDiff(a: readonly string[], b: readonly string[]): boolean {
  return a.length > DIFF_MAX_LINES || b.length > DIFF_MAX_LINES;
}

/** Split text into lines on LF or CRLF. A lone CR stays in its line. Empty text has no lines. */
export function splitLines(text: string): string[] {
  if (text === '') return [];
  return text.split(/\r?\n/);
}

/**
 * Work budget (diagonal steps) for one diff. Typical responses need a tiny
 * fraction of it; a pathological pair (two long, unrelated texts built from a
 * handful of repeated lines) would otherwise take seconds. Past the budget the
 * remaining regions are reported as plain delete + add: still a correct diff,
 * just not a minimal one, and flagged so the view can say so.
 */
const WORK_BUDGET = 30_000_000;

/**
 * Mark matched positions of `a` and `b` (integer sequences) with a
 * linear-space Myers search. Returns false when the work budget ran out.
 */
function matchSequences(a: Int32Array, b: Int32Array, matchA: Uint8Array, matchB: Uint8Array): boolean {
  let work = 0;
  let exact = true;
  const recurse = (aLo: number, aHi: number, bLo: number, bHi: number): void => {
    while (aLo < aHi && bLo < bHi && a[aLo] === b[bLo]) {
      matchA[aLo++] = 1;
      matchB[bLo++] = 1;
    }
    while (aLo < aHi && bLo < bHi && a[aHi - 1] === b[bHi - 1]) {
      matchA[--aHi] = 1;
      matchB[--bHi] = 1;
    }
    if (aLo === aHi || bLo === bHi) return;

    const n = aHi - aLo;
    const m = bHi - bLo;
    const delta = n - m;
    const odd = (delta & 1) !== 0;
    const max = Math.ceil((n + m) / 2);
    const off = max + Math.abs(delta) + 2;
    const vf = new Int32Array(2 * off + 1);
    const vb = new Int32Array(2 * off + 1);
    vf[off + 1] = 0;
    vb[off + delta + 1] = n + 1;

    // The middle snake, in coordinates relative to (aLo, bLo).
    let sx = -1;
    let sy = -1;
    let su = -1;
    let sv = -1;
    search: for (let d = 0; d <= max; d++) {
      work += 2 * d + 2;
      if (work > WORK_BUDGET) {
        exact = false;
        return;
      }
      for (let k = -d; k <= d; k += 2) {
        let x = k === -d || (k !== d && vf[off + k - 1] < vf[off + k + 1]) ? vf[off + k + 1] : vf[off + k - 1] + 1;
        let y = x - k;
        const x0 = x;
        const y0 = y;
        while (x < n && y < m && a[aLo + x] === b[bLo + y]) {
          x++;
          y++;
          work++;
        }
        vf[off + k] = x;
        if (odd && k >= delta - (d - 1) && k <= delta + (d - 1) && x >= vb[off + k]) {
          sx = x0;
          sy = y0;
          su = x;
          sv = y;
          break search;
        }
      }
      for (let kk = -d; kk <= d; kk += 2) {
        const k = kk + delta;
        let x =
          kk === -d || (kk !== d && vb[off + k + 1] - 1 < vb[off + k - 1]) ? vb[off + k + 1] - 1 : vb[off + k - 1];
        let y = x - k;
        const x1 = x;
        const y1 = y;
        while (x > 0 && y > 0 && a[aLo + x - 1] === b[bLo + y - 1]) {
          x--;
          y--;
          work++;
        }
        vb[off + k] = x;
        if (!odd && k >= -d && k <= d && x <= vf[off + k]) {
          sx = x;
          sy = y;
          su = x1;
          sv = y1;
          break search;
        }
      }
    }
    // Unreachable for valid input; guard against a split that makes no progress.
    if (sx < 0 || (su === sx && ((sx === 0 && sy === 0) || (sx === n && sy === m)))) return;
    for (let i = 0; i < su - sx; i++) {
      matchA[aLo + sx + i] = 1;
      matchB[bLo + sy + i] = 1;
    }
    recurse(aLo, aLo + sx, bLo, bLo + sy);
    recurse(aLo + su, aHi, bLo + sv, bHi);
  };
  recurse(0, a.length, 0, b.length);
  return exact;
}

/**
 * Diff two line arrays: every line of `a` and `b` appears once, in order, as
 * `same` (in both), `del` (only in `a`) or `add` (only in `b`), with the
 * fewest adds and deletes. Throws RangeError above DIFF_MAX_LINES per side.
 */
export function diffLines(a: readonly string[], b: readonly string[]): DiffLine[] {
  return diffLinesDetailed(a, b).lines;
}

/** `diffLines` plus whether the result is minimal (false once the work budget ran out). */
export function diffLinesDetailed(a: readonly string[], b: readonly string[]): { lines: DiffLine[]; exact: boolean } {
  if (tooLargeToDiff(a, b)) throw new RangeError(`diff is bounded to ${DIFF_MAX_LINES} lines per side`);

  // Intern lines to integers.
  const ids = new Map<string, number>();
  const intern = (s: string): number => {
    let v = ids.get(s);
    if (v === undefined) {
      v = ids.size;
      ids.set(s, v);
    }
    return v;
  };
  const ai = Int32Array.from(a, intern);
  const bi = Int32Array.from(b, intern);

  // Lines that occur on both sides are the only candidates for a match.
  const inA = new Uint8Array(ids.size);
  const inB = new Uint8Array(ids.size);
  for (const v of ai) inA[v] = 1;
  for (const v of bi) inB[v] = 1;
  const keepA: number[] = [];
  const keepB: number[] = [];
  for (let i = 0; i < ai.length; i++) if (inB[ai[i]]) keepA.push(i);
  for (let j = 0; j < bi.length; j++) if (inA[bi[j]]) keepB.push(j);

  const ca = Int32Array.from(keepA, (i) => ai[i]);
  const cb = Int32Array.from(keepB, (j) => bi[j]);
  const cma = new Uint8Array(ca.length);
  const cmb = new Uint8Array(cb.length);
  const exact = matchSequences(ca, cb, cma, cmb);

  const matchA = new Uint8Array(a.length);
  const matchB = new Uint8Array(b.length);
  for (let i = 0; i < keepA.length; i++) if (cma[i]) matchA[keepA[i]] = 1;
  for (let j = 0; j < keepB.length; j++) if (cmb[j]) matchB[keepB[j]] = 1;

  // Matches form a monotone pairing: walk both sides in step.
  const out: DiffLine[] = [];
  let i = 0;
  let j = 0;
  while (i < a.length || j < b.length) {
    if (i < a.length && !matchA[i]) out.push({ kind: 'del', text: a[i++] });
    else if (j < b.length && !matchB[j]) out.push({ kind: 'add', text: b[j++] });
    else {
      out.push({ kind: 'same', text: a[i] });
      i++;
      j++;
    }
  }
  return { lines: out, exact };
}

/** One rendered diff row: a line (with its index in the diff) or a fold of unchanged lines. */
export type DiffRow = (DiffLine & { index: number }) | { kind: 'fold'; count: number; from: number };

/**
 * Fold runs of unchanged lines, keeping `context` lines next to each change.
 * A run is folded only when that hides more than one line.
 */
export function collapseUnchanged(diff: readonly DiffLine[], context = 3): DiffRow[] {
  const rows: DiffRow[] = [];
  let i = 0;
  while (i < diff.length) {
    if (diff[i].kind !== 'same') {
      rows.push({ ...diff[i], index: i });
      i++;
      continue;
    }
    let end = i;
    while (end < diff.length && diff[end].kind === 'same') end++;
    const keepHead = i === 0 ? 0 : context;
    const keepTail = end === diff.length ? 0 : context;
    const hidden = end - i - keepHead - keepTail;
    if (hidden > 1) {
      for (let k = i; k < i + keepHead; k++) rows.push({ ...diff[k], index: k });
      rows.push({ kind: 'fold', count: hidden, from: i + keepHead });
      for (let k = end - keepTail; k < end; k++) rows.push({ ...diff[k], index: k });
    } else {
      for (let k = i; k < end; k++) rows.push({ ...diff[k], index: k });
    }
    i = end;
  }
  return rows;
}

/** Old/new line numbers per diff line (0 where the line is absent on that side). */
export function lineNumbers(diff: readonly DiffLine[]): { a: Int32Array; b: Int32Array } {
  const a = new Int32Array(diff.length);
  const b = new Int32Array(diff.length);
  let na = 0;
  let nb = 0;
  diff.forEach((l, i) => {
    if (l.kind !== 'add') a[i] = ++na;
    if (l.kind !== 'del') b[i] = ++nb;
  });
  return { a, b };
}
