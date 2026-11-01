import { describe, expect, it } from 'vitest';
import { FIND_LIMIT, findAll, foldCase, nextMatch, prevMatch } from './find';

describe('findAll', () => {
  it('finds every non-overlapping match in Latin-1 text', () => {
    expect(findAll('abcabcab', 'ab', { caseSensitive: true })).toEqual([0, 3, 6]);
    expect(findAll('aaaa', 'aa', { caseSensitive: true })).toEqual([0, 2]);
    expect(findAll('\x00\xff\r\n\xff', '\xff', { caseSensitive: true })).toEqual([1, 4]);
  });

  it('is case-insensitive on demand, including Latin-1 letters', () => {
    expect(findAll('Set-Cookie: SET-cookie', 'set-cookie', { caseSensitive: false })).toEqual([0, 12]);
    expect(findAll('Set-Cookie', 'set-cookie', { caseSensitive: true })).toEqual([]);
    expect(findAll('ÀÉ àé', 'àé', { caseSensitive: false })).toEqual([0, 3]);
  });

  it('returns nothing for an empty needle', () => {
    expect(findAll('abc', '', { caseSensitive: false })).toEqual([]);
  });
});

describe('nextMatch / prevMatch', () => {
  it('wraps around both ways', () => {
    expect(nextMatch(3, 0)).toBe(1);
    expect(nextMatch(3, 2)).toBe(0);
    expect(prevMatch(3, 0)).toBe(2);
    expect(prevMatch(3, 1)).toBe(0);
    expect(nextMatch(0, 0)).toBe(-1);
    expect(prevMatch(0, -1)).toBe(-1);
    expect(nextMatch(3, -1)).toBe(0);
  });
});

describe('find limits', () => {
  it('stops at FIND_LIMIT + 1 so the caller can show "10000+"', () => {
    const text = 'a'.repeat(1024 * 1024);
    const t0 = performance.now();
    const hits = findAll(text, 'a', { caseSensitive: false });
    expect(hits.length).toBe(FIND_LIMIT + 1);
    expect(FIND_LIMIT).toBe(10_000);
    expect(performance.now() - t0).toBeLessThan(1000);
  });

  it('accepts a pre-folded haystack (same offsets as folding per search)', () => {
    const text = 'ÀbC àBc';
    const folded = foldCase(text);
    expect(folded.length).toBe(text.length);
    expect(findAll(text, 'àbc', { caseSensitive: false, folded })).toEqual([0, 4]);
  });
});
