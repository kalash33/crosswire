import { describe, expect, it } from 'vitest';
import { DIFF_MAX_LINES, collapseUnchanged, diffLines, diffLinesDetailed, splitLines, tooLargeToDiff, type DiffLine } from './lines';

/** Reference LCS length by dynamic programming (small inputs only). */
function lcsLength(a: string[], b: string[]): number {
  const dp = Array.from({ length: a.length + 1 }, () => new Array<number>(b.length + 1).fill(0));
  for (let i = a.length - 1; i >= 0; i--)
    for (let j = b.length - 1; j >= 0; j--)
      dp[i][j] = a[i] === b[j] ? dp[i + 1][j + 1] + 1 : Math.max(dp[i + 1][j], dp[i][j + 1]);
  return dp[0][0];
}

/** The diff must rebuild both sides exactly. */
function sides(d: DiffLine[]): { a: string[]; b: string[] } {
  return {
    a: d.filter((l) => l.kind !== 'add').map((l) => l.text),
    b: d.filter((l) => l.kind !== 'del').map((l) => l.text),
  };
}

describe('diffLines', () => {
  it('marks added, removed and unchanged lines', () => {
    const d = diffLines(['a', 'b', 'c'], ['a', 'x', 'c', 'd']);
    expect(d).toEqual([
      { kind: 'same', text: 'a' },
      { kind: 'del', text: 'b' },
      { kind: 'add', text: 'x' },
      { kind: 'same', text: 'c' },
      { kind: 'add', text: 'd' },
    ]);
  });

  it('identical input is all unchanged', () => {
    const a = ['HTTP/1.1 200 OK', 'a: b', '', 'body'];
    expect(diffLines(a, [...a])).toEqual(a.map((text) => ({ kind: 'same', text })));
  });

  it('handles empty sides', () => {
    expect(diffLines([], [])).toEqual([]);
    expect(diffLines([], ['x', 'y'])).toEqual([
      { kind: 'add', text: 'x' },
      { kind: 'add', text: 'y' },
    ]);
    expect(diffLines(['x'], [])).toEqual([{ kind: 'del', text: 'x' }]);
  });

  it('completely different sides are a delete then an add', () => {
    expect(diffLines(['a', 'b'], ['c', 'd'])).toEqual([
      { kind: 'del', text: 'a' },
      { kind: 'del', text: 'b' },
      { kind: 'add', text: 'c' },
      { kind: 'add', text: 'd' },
    ]);
  });

  it('is a minimal diff (matches the reference LCS) on random input', () => {
    let seed = 7;
    const rnd = (n: number) => {
      seed = (seed * 1103515245 + 12345) & 0x7fffffff;
      return seed % n;
    };
    for (let round = 0; round < 300; round++) {
      const alphabet = 1 + rnd(5);
      const a = Array.from({ length: rnd(25) }, () => String(rnd(alphabet)));
      const b = Array.from({ length: rnd(25) }, () => String(rnd(alphabet)));
      const d = diffLines(a, b);
      expect(sides(d)).toEqual({ a, b });
      expect(d.filter((l) => l.kind === 'same').length).toBe(lcsLength(a, b));
    }
  });

  it('keeps hostile text verbatim', () => {
    const x = '<img src=x onerror=alert(1)>';
    expect(diffLines([x], [x, `${x}2`])).toEqual([
      { kind: 'same', text: x },
      { kind: 'add', text: `${x}2` },
    ]);
  });

  it('is bounded to DIFF_MAX_LINES per side', () => {
    expect(DIFF_MAX_LINES).toBe(20000);
    const ok = new Array<string>(DIFF_MAX_LINES).fill('x');
    const big = new Array<string>(DIFF_MAX_LINES + 1).fill('x');
    expect(tooLargeToDiff(ok, ok)).toBe(false);
    expect(tooLargeToDiff(big, [])).toBe(true);
    expect(tooLargeToDiff([], big)).toBe(true);
    expect(() => diffLines(big, [])).toThrow(RangeError);
  });

  it('diffs two 20k-line sides with scattered edits quickly', () => {
    const a = Array.from({ length: DIFF_MAX_LINES }, (_, i) => `line ${i % 97} ${i}`);
    const b = a.map((l, i) => (i % 500 === 0 ? `${l} changed` : l));
    const t = performance.now();
    const d = diffLines(a, b);
    expect(performance.now() - t).toBeLessThan(2000);
    expect(sides(d)).toEqual({ a, b });
    expect(d.filter((l) => l.kind === 'add').length).toBe(40);
  });
});

describe('splitLines', () => {
  it('splits on LF and CRLF, keeping a lone CR', () => {
    expect(splitLines('a\r\nb\nc\rd')).toEqual(['a', 'b', 'c\rd']);
    expect(splitLines('')).toEqual([]);
    expect(splitLines('a\n')).toEqual(['a', '']);
  });
});

describe('collapseUnchanged', () => {
  const same = (n: number, p = 's') => Array.from({ length: n }, (_, i) => ({ kind: 'same' as const, text: `${p}${i}` }));

  it('folds long unchanged runs, keeping context around changes', () => {
    const d: DiffLine[] = [...same(10, 'a'), { kind: 'add', text: 'x' }, ...same(10, 'b')];
    const rows = collapseUnchanged(d, 3);
    expect(rows.map((r) => (r.kind === 'fold' ? `fold ${r.count}` : r.text))).toEqual([
      'fold 7', 'a7', 'a8', 'a9', 'x', 'b0', 'b1', 'b2', 'fold 7',
    ]);
  });

  it('leaves short runs alone and folds an all-unchanged diff into one row', () => {
    const d: DiffLine[] = [{ kind: 'del', text: 'x' }, ...same(4), { kind: 'add', text: 'y' }];
    expect(collapseUnchanged(d, 3)).toHaveLength(6);
    expect(collapseUnchanged(same(5), 3)).toEqual([{ kind: 'fold', count: 5, from: 0 }]);
  });
});

describe('diff work budget', () => {
  it('a pathological pair finishes fast with a correct (non-minimal) diff', () => {
    // mulberry32: a well-mixed seeded generator.
    let seed = 11;
    const r = () => {
      seed = (seed + 0x6d2b79f5) | 0;
      let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
      t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
      return String(((t ^ (t >>> 14)) >>> 0) % 10);
    };
    const a = Array.from({ length: DIFF_MAX_LINES }, r);
    const b = Array.from({ length: DIFF_MAX_LINES }, r);
    const t = performance.now();
    const { lines, exact } = diffLinesDetailed(a, b);
    expect(performance.now() - t).toBeLessThan(5000);
    expect(exact).toBe(false);
    expect(sides(lines)).toEqual({ a, b });
  });

  it('ordinary diffs are exact', () => {
    expect(diffLinesDetailed(['a', 'b'], ['a', 'c']).exact).toBe(true);
  });
});
