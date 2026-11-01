import { describe, expect, it } from 'vitest';
import { contentTypeOf, defaultTab, detectPretty, PRETTY_LIMIT } from './detect';

const enc = (s: string) => Uint8Array.from(s, (c) => c.charCodeAt(0));
const head = (ct: string | null) =>
  enc(`HTTP/1.1 200 OK\r\n${ct === null ? '' : `Content-Type: ${ct}\r\n`}X: y\r\n\r\n`);

describe('contentTypeOf', () => {
  it('reads the media type from a raw head, case-insensitively, without parameters', () => {
    expect(contentTypeOf(head('Application/JSON; charset=utf-8'))).toBe('application/json');
    expect(contentTypeOf(enc('HTTP/1.1 200 OK\r\ncontent-type:text/html\r\n\r\n'))).toBe('text/html');
    expect(contentTypeOf(head(null))).toBe('');
    expect(contentTypeOf(null)).toBe('');
  });
});

describe('detectPretty', () => {
  it('detects JSON by content type (application/json and +json)', () => {
    expect(detectPretty(head('application/json'), enc('not json'))).toEqual({
      kind: 'json',
      reason: 'type',
      tooLarge: false,
    });
    expect(detectPretty(null, enc('{}'), 'application/problem+json; charset=utf-8').reason).toBe('type');
  });

  it('detects JSON by a successful parse when the type says otherwise', () => {
    expect(detectPretty(head('text/html'), enc(' {"a":[1,2]}\n'))).toEqual({
      kind: 'json',
      reason: 'parse',
      tooLarge: false,
    });
    // UTF-8 text inside the Latin-1 byte view still parses.
    expect(detectPretty(null, new TextEncoder().encode('["é","日本"]')).kind).toBe('json');
  });

  it('does not call scalars or broken JSON JSON', () => {
    expect(detectPretty(null, enc('123')).kind).toBe('none');
    expect(detectPretty(null, enc('"str"')).kind).toBe('none');
    expect(detectPretty(null, enc('{"a":')).kind).toBe('none');
    expect(detectPretty(null, enc('')).kind).toBe('none');
  });

  it('detects a urlencoded form by type', () => {
    expect(detectPretty(null, enc('a=1&b=2'), 'application/x-www-form-urlencoded')).toEqual({
      kind: 'form',
      reason: 'type',
      tooLarge: false,
    });
  });

  it('falls back to the query of a request target', () => {
    expect(detectPretty(null, enc(''), undefined, '/search?q=1&x')).toEqual({
      kind: 'query',
      reason: 'type',
      tooLarge: false,
    });
    expect(detectPretty(null, enc(''), undefined, '/search').kind).toBe('none');
    // A JSON body wins over the query (the query table is shown above it).
    expect(detectPretty(null, enc('{}'), 'application/json', '/a?b=1').kind).toBe('json');
  });

  it('guards the size: over 2 MiB is too large and is never parsed', () => {
    const big = new Uint8Array(PRETTY_LIMIT + 1).fill(0x20);
    big[0] = 0x5b; // '['
    big[big.length - 1] = 0x5d; // ']'
    expect(detectPretty(null, big, 'application/json')).toEqual({ kind: 'json', reason: 'type', tooLarge: true });
    // By-parse detection is skipped entirely over the limit.
    expect(detectPretty(null, big).kind).toBe('none');
    expect(detectPretty(null, new Uint8Array(PRETTY_LIMIT).fill(0x20), 'application/json').tooLarge).toBe(false);
  });
});

describe('defaultTab', () => {
  it('opens Pretty for typed JSON, forms and queries, Raw otherwise', () => {
    expect(defaultTab({ kind: 'json', reason: 'type', tooLarge: false }, '')).toBe('pretty');
    expect(defaultTab({ kind: 'form', reason: 'type', tooLarge: false }, '')).toBe('pretty');
    expect(defaultTab({ kind: 'query', reason: 'type', tooLarge: false }, '')).toBe('pretty');
    expect(defaultTab({ kind: 'none', reason: 'none', tooLarge: false }, '')).toBe('raw');
    expect(defaultTab({ kind: 'json', reason: 'type', tooLarge: true }, 'application/json')).toBe('raw');
  });

  it('keeps Raw for parse-detected JSON whose declared type is something else', () => {
    expect(defaultTab({ kind: 'json', reason: 'parse', tooLarge: false }, 'text/html')).toBe('raw');
    expect(defaultTab({ kind: 'json', reason: 'parse', tooLarge: false }, '')).toBe('pretty');
    expect(defaultTab({ kind: 'json', reason: 'parse', tooLarge: false }, 'text/plain')).toBe('pretty');
  });
});
