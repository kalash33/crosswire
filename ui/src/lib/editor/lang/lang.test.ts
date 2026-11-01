import { describe, expect, it } from 'vitest';
import { StringStream, type StreamParser } from '@codemirror/language';
import { jsonTokens } from './json';
import { httpTokens } from './http';
import { markupTokens } from './markup';
import { TOKENS, type Token } from './tokens';

// Run a tokenizer over a document the way StreamLanguage does: one line at a
// time, `blankLine` for empty lines, state carried across lines.
function tokenize<S>(p: StreamParser<S>, doc: string): [string, Token | null][] {
  const state = p.startState!(2);
  const out: [string, Token | null][] = [];
  for (const line of doc.split('\n')) {
    if (line === '') {
      p.blankLine?.(state, 2);
      continue;
    }
    const s = new StringStream(line, 4, 2);
    while (!s.eol()) {
      s.start = s.pos;
      const t = p.token(s, state) as Token | null;
      if (s.pos <= s.start) throw new Error(`no progress at ${s.pos} in ${JSON.stringify(line)}`);
      out.push([line.slice(s.start, s.pos), t]);
    }
  }
  return out;
}
const styled = (r: [string, Token | null][]) => r.filter(([, t]) => t !== null);

describe('json tokenizer', () => {
  it('classes keys, strings, numbers, keywords and punctuation', () => {
    expect(styled(tokenize(jsonTokens, '{"a\\"b" : "v", "n": -1.5e3, "k": [true, null]}'))).toEqual([
      ['{', 'punctuation'],
      ['"a\\"b"', 'propertyName'],
      [':', 'punctuation'],
      ['"v"', 'string'],
      [',', 'punctuation'],
      ['"n"', 'propertyName'],
      [':', 'punctuation'],
      ['-1.5e3', 'number'],
      [',', 'punctuation'],
      ['"k"', 'propertyName'],
      [':', 'punctuation'],
      ['[', 'punctuation'],
      ['true', 'keyword'],
      [',', 'punctuation'],
      ['null', 'keyword'],
      [']', 'punctuation'],
      ['}', 'punctuation'],
    ]);
  });
});

describe('http tokenizer', () => {
  it('classes the start line, header names and values, and stops at the body', () => {
    const r = styled(tokenize(httpTokens, 'HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n folded\r\n\r\n{"a":1}'));
    expect(r).toEqual([
      ['HTTP/1.1 200 OK\r', 'statusLine'],
      ['Content-Type', 'headerName'],
      [':', 'punctuation'],
      ['text/html\r', 'headerValue'],
      ['folded\r', 'headerValue'],
    ]);
  });
});

describe('markup tokenizer', () => {
  it('classes tags, attributes, values and comments, across lines', () => {
    const r = styled(tokenize(markupTokens, '<a href="x\ny" b=c>t</a><!-- c\n-->'));
    expect(r).toEqual([
      ['<', 'punctuation'],
      ['a', 'tagName'],
      ['href', 'attributeName'],
      ['=', 'punctuation'],
      ['"x', 'attributeValue'],
      ['y"', 'attributeValue'],
      ['b', 'attributeName'],
      ['=', 'punctuation'],
      ['c', 'attributeValue'],
      ['>', 'punctuation'],
      ['</', 'punctuation'],
      ['a', 'tagName'],
      ['>', 'punctuation'],
      ['<!-- c', 'comment'],
      ['-->', 'comment'],
    ]);
  });
});

// Hostile input: every byte value, unterminated constructs and the XSS corpus
// shapes. A tokenizer must always advance, never throw, and only ever return
// a class from the fixed set (or null).
const BYTES = Array.from({ length: 256 }, (_, i) => String.fromCharCode(i)).join('');
const HOSTILE = [
  BYTES,
  BYTES.split('').reverse().join(''),
  '"unterminated',
  '{"a":"\\',
  '<',
  '</',
  '<a b="',
  '<!--',
  '<!doctype html><script>alert(1)</script>',
  '<img src=x onerror="alert(1)">',
  '"><svg onload=alert(1)>',
  "' onmouseover='alert(1)",
  '${alert(1)}{{constructor.constructor("alert(1)")()}}',
  ':::',
  'HTTP/1.1\r\n:\r\n\r\n\r\n',
  '\\u0000\\ud800',
  '-',
  '1e',
  'tru',
  '\u2028\u2029\ufeff',
];

describe('tokenizers on hostile input', () => {
  for (const [name, p] of [
    ['json', jsonTokens],
    ['http', httpTokens],
    ['markup', markupTokens],
  ] as const) {
    it(`${name} never throws, always advances and returns only fixed classes`, () => {
      for (const doc of [...HOSTILE, HOSTILE.join('\n'), BYTES.repeat(4)]) {
        const r = tokenize(p as StreamParser<unknown>, doc);
        for (const [, t] of r) if (t !== null) expect(TOKENS).toContain(t);
        // Every character of every line is covered exactly once.
        expect(r.map(([s]) => s).join('')).toBe(doc.split('\n').join(''));
      }
    });
  }
});
