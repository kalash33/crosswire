import { describe, expect, it } from 'vitest';
import { parseHeadLines } from './headers';

const enc = (s: string) => Uint8Array.from(s, (c) => c.charCodeAt(0));

describe('parseHeadLines', () => {
  it('splits a CRLF head into start line and headers, stopping at the blank line', () => {
    const r = parseHeadLines(enc('GET / HTTP/1.1\r\nHost: a.test\r\nAccept: */*\r\n\r\nBODY: no\r\n'));
    expect(r.start).toBe('GET / HTTP/1.1');
    expect(r.headers).toEqual([
      { name: 'Host', value: 'a.test' },
      { name: 'Accept', value: '*/*' },
    ]);
    expect(r.malformed).toEqual([]);
  });

  it('accepts bare LF and lone CR line ends', () => {
    expect(parseHeadLines(enc('HTTP/1.1 200 OK\nA: 1\nB: 2\n\n')).headers.map((h) => h.name)).toEqual(['A', 'B']);
    const cr = parseHeadLines(enc('HTTP/1.1 200 OK\rA: 1\rB: 2\r\r'));
    expect(cr.start).toBe('HTTP/1.1 200 OK');
    expect(cr.headers).toEqual([
      { name: 'A', value: '1' },
      { name: 'B', value: '2' },
    ]);
  });

  it('never renders CR or LF in any output string', () => {
    const r = parseHeadLines(enc('GET /\r HTTP/1.1\r\nX: a\rb\r\n\tfold\r\n\r\n'));
    const all = [r.start, ...r.headers.flatMap((h) => [h.name, h.value]), ...r.malformed].join('|');
    expect(all).not.toMatch(/[\r\n]/);
  });

  it('joins obs-fold continuation lines onto the previous value', () => {
    const r = parseHeadLines(enc('GET / HTTP/1.1\r\nX-Long: one\r\n  two\r\n\tthree\r\nY: z\r\n\r\n'));
    expect(r.headers).toEqual([
      { name: 'X-Long', value: 'one two three' },
      { name: 'Y', value: 'z' },
    ]);
  });

  it('puts a line without a colon (and a leading fold) in malformed', () => {
    const r = parseHeadLines(enc('GET / HTTP/1.1\r\n folded-first\r\nnocolon\r\nA: b:c\r\n\r\n'));
    expect(r.malformed).toEqual([' folded-first', 'nocolon']);
    expect(r.headers).toEqual([{ name: 'A', value: 'b:c' }]);
  });

  it('shows 0x80-0xFF bytes 1:1 via Latin-1, without decoding', () => {
    const bytes = new Uint8Array([...enc('HTTP/1.1 200 OK\r\nX: '), 0x80, 0xe9, 0xff, ...enc('\r\n\r\n')]);
    const r = parseHeadLines(bytes);
    expect(r.headers[0].value).toBe('\u0080éÿ');
  });

  it('never throws on empty or odd input', () => {
    expect(parseHeadLines(new Uint8Array())).toEqual({ start: '', headers: [], malformed: [] });
    expect(() => parseHeadLines(enc(':\r\n:::\r\n\r\r\n'))).not.toThrow();
    expect(parseHeadLines(enc('S\r\n: v\r\n')).headers).toEqual([{ name: '', value: 'v' }]);
  });
});
