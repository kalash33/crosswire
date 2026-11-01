import { describe, expect, it } from 'vitest';
import { findAll, nextMatch, prevMatch } from './find';

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
