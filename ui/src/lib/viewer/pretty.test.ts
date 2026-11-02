import { describe, expect, it } from 'vitest';
import { countPairs, isValidUtf8, parseForm, parseQuery, prettyJson } from './pretty';
import { PRETTY_LIMIT } from './detect';

const enc = (s: string) => Uint8Array.from(s, (c) => c.charCodeAt(0));
const utf8 = (s: string) => new TextEncoder().encode(s);

describe('prettyJson', () => {
  it('matches 2-space JSON.stringify on ordinary input', () => {
    const src = '{"a":1,"b":[true,false,null,{"c":"x"}],"d":{},"e":[],"f":-1.5}';
    expect(prettyJson(enc(src))).toBe(JSON.stringify(JSON.parse(src), null, 2));
  });

  it('is stable: formatting its own output is a no-op', () => {
    const once = prettyJson(enc('[{"k" : [ 1 , 2 ] }]'))!;
    expect(prettyJson(utf8(once))).toBe(once);
  });

  it('keeps the recorded text: big numbers, escapes, key order and duplicate keys', () => {
    const out = prettyJson(enc('{"2":1,"1":12345678901234567890,"s":"a\\u003cb\\"","2":0}'))!;
    expect(out).toBe('{\n  "2": 1,\n  "1": 12345678901234567890,\n  "s": "a\\u003cb\\"",\n  "2": 0\n}');
  });

  it('decodes UTF-8 and keeps structural characters inside strings intact', () => {
    expect(prettyJson(utf8('{"a":"é{,}[:]"}'))).toBe('{\n  "a": "é{,}[:]"\n}');
  });

  it('falls back (null) on invalid JSON and over the 2 MiB guard', () => {
    expect(prettyJson(enc('{"a":'))).toBeNull();
    expect(prettyJson(enc(''))).toBeNull();
    const big = new Uint8Array(PRETTY_LIMIT + 1).fill(0x20);
    big[0] = 0x30;
    expect(prettyJson(big)).toBeNull();
  });
});

describe('parseForm', () => {
  it('URL-decodes keys and values, + as space, UTF-8 percent sequences', () => {
    expect(parseForm(enc('a=1&b=two+words&c%20d=%C3%A9&flag&&e='))).toEqual([
      { key: 'a', value: '1' },
      { key: 'b', value: 'two words' },
      { key: 'c d', value: 'é' },
      { key: 'flag', value: '' },
      { key: 'e', value: '' },
    ]);
  });

  it('keeps malformed percent sequences literally and splits at the first =', () => {
    expect(parseForm(enc('x=%zz%4&y=a=b%'))).toEqual([
      { key: 'x', value: '%zz%4' },
      { key: 'y', value: 'a=b%' },
    ]);
  });
});

describe('parseQuery', () => {
  it('parses the part after ? and before #', () => {
    expect(parseQuery('/p?q=1+2&r=%3Cb%3E#frag')).toEqual([
      { key: 'q', value: '1 2' },
      { key: 'r', value: '<b>' },
    ]);
    expect(parseQuery('/p')).toEqual([]);
    expect(parseQuery('http://h/x?=v')).toEqual([{ key: '', value: 'v' }]);
  });
});

describe('limits and UTF-8', () => {
  it('parseForm stops at a row limit and countPairs counts every non-empty pair', () => {
    const body = enc('a=&'.repeat(5000));
    expect(parseForm(body, 1001)).toHaveLength(1001);
    expect(countPairs(body)).toBe(5000);
    expect(countPairs(enc('&&a=1&&b&'))).toBe(2);
    expect(countPairs(enc(''))).toBe(0);
  });

  it('isValidUtf8 tells strict UTF-8 from bytes that decode with replacements', () => {
    expect(isValidUtf8(utf8('{"a":"é日本"}'))).toBe(true);
    expect(isValidUtf8(Uint8Array.from([0x7b, 0xff, 0x7d]))).toBe(false);
    expect(isValidUtf8(Uint8Array.from([0xc3]))).toBe(false);
  });
});
