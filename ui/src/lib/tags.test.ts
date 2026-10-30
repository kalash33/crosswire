import { describe, expect, it } from 'vitest';
import { addTag } from './tags';

describe('addTag', () => {
  it('rejects_empty', () => {
    expect(addTag([], '   ')).toEqual({ ok: false, error: 'tag is empty' });
  });
  it('rejects_too_long', () => {
    expect(addTag([], 'a'.repeat(65))).toEqual({ ok: false, error: 'tag is longer than 64 characters' });
    expect(addTag([], 'a'.repeat(64)).ok).toBe(true);
  });
  it('rejects_control_chars', () => {
    expect(addTag([], 'a\u0007b')).toEqual({ ok: false, error: 'tag contains control characters' });
    expect(addTag([], 'a\u007fb')).toEqual({ ok: false, error: 'tag contains control characters' });
  });
  it('rejects_duplicates', () => {
    expect(addTag(['a'], 'a')).toEqual({ ok: false, error: 'tag already present' });
  });
  it('rejects_33rd_tag', () => {
    const full = Array.from({ length: 32 }, (_, i) => `t${i}`);
    expect(addTag(full, 'x')).toEqual({ ok: false, error: 'at most 32 tags' });
    expect(addTag(full.slice(1), 'x').ok).toBe(true);
  });
  it('addTag_trims_and_appends', () => {
    expect(addTag(['b'], ' a ')).toEqual({ ok: true, tags: ['b', 'a'] });
  });
  it('addTag_is_case_sensitive', () => {
    expect(addTag(['A'], 'a')).toEqual({ ok: true, tags: ['A', 'a'] });
  });
});
