import { describe, expect, it } from 'vitest';
import { fromTextareaText, toTextareaText } from './textarea';

describe('textarea mapping', () => {
  it('round_trips_every_byte', () => {
    const all = Uint8Array.from({ length: 256 }, (_, i) => i);
    const r = fromTextareaText(toTextareaText(all));
    expect(Array.from(r.bytes)).toEqual(Array.from(all));
    expect(r.nonLatin1).toBe(false);
  });

  it('cr_shown_as_sentinel', () => {
    expect(toTextareaText(Uint8Array.from([0x61, 0x0d, 0x0a, 0x62]))).toBe('a␍\nb');
  });

  it('wide_text_is_utf8', () => {
    const r = fromTextareaText('€␍');
    expect(Array.from(r.bytes)).toEqual([0xe2, 0x82, 0xac, 0x0d]);
    expect(r.nonLatin1).toBe(true);
  });
});
