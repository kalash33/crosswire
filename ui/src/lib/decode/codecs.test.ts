import { describe, expect, it } from 'vitest';
import { apply, CODECS, jwtView, type Direction } from './codecs';

const enc = (s: string) => new TextEncoder().encode(s);
const all256 = Uint8Array.from({ length: 256 }, (_, i) => i);
const b64url = (s: string) =>
  btoa(s).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');

describe('codecs', () => {
  for (const codec of CODECS) {
    it(`${codec}: roundtrip_all_bytes`, () => {
      const e = apply(codec, 'encode', all256);
      const d = apply(codec, 'decode', e.bytes);
      expect(Array.from(d.bytes)).toEqual(Array.from(all256));
      expect(d.warning).toBeNull();
    });
  }

  it('base64_accepts_urlsafe_and_unpadded', () => {
    const std = apply('base64', 'decode', enc('+/+/Pw==')).bytes;
    const url = apply('base64', 'decode', enc('-_-_Pw')).bytes;
    expect(Array.from(url)).toEqual(Array.from(std));
    expect(apply('base64', 'decode', enc('-_-_Pw')).warning).toBeNull();
  });

  it('base64_invalid_chars_warn', () => {
    const r = apply('base64', 'decode', enc('ab!c'));
    expect(r.warning).toBe('base64: 1 invalid characters skipped');
  });

  it('hex_odd_length_warns', () => {
    const r = apply('hex', 'decode', enc('abc'));
    expect(Array.from(r.bytes)).toEqual([0xab]);
    expect(r.warning).toBe('hex: odd length, last nibble dropped');
  });

  it('hex_accepts_0x_and_whitespace', () => {
    const r = apply('hex', 'decode', enc('0xDE ad\nBE\tef'));
    expect(Array.from(r.bytes)).toEqual([0xde, 0xad, 0xbe, 0xef]);
    expect(r.warning).toBeNull();
  });

  it('url_plus_is_literal', () => {
    const r = apply('url', 'decode', enc('a+b'));
    expect(Array.from(r.bytes)).toEqual(Array.from(enc('a+b')));
    expect(r.warning).toBeNull();
  });

  it('url_malformed_percent_kept', () => {
    const r = apply('url', 'decode', enc('%zz%4'));
    expect(Array.from(r.bytes)).toEqual(Array.from(enc('%zz%4')));
    expect(r.warning).toBe('url: 2 malformed % sequences kept literally');
  });

  it('html_numeric_and_named', () => {
    const r = apply('html', 'decode', enc('&lt;&#65;&#x42;&nbsp;'));
    expect(Array.from(r.bytes)).toEqual([0x3c, 0x41, 0x42, 0xc2, 0xa0]);
    expect(r.warning).toBeNull();
  });

  it('html_unknown_entity_kept', () => {
    const r = apply('html', 'decode', enc('&bogus;'));
    expect(Array.from(r.bytes)).toEqual(Array.from(enc('&bogus;')));
    expect(r.warning).toBe('html: 1 unknown entities kept literally');
  });

  it('html_encode_escapes_five', () => {
    const r = apply('html', 'encode', enc(`&<>"'`));
    expect(new TextDecoder().decode(r.bytes)).toBe('&amp;&lt;&gt;&quot;&#39;');
  });

  it('apply_never_throws_on_random_bytes', () => {
    for (let n = 0; n < 200; n++) {
      const input = new Uint8Array(Math.floor(Math.random() * 64));
      for (let i = 0; i < input.length; i++) input[i] = Math.floor(Math.random() * 256);
      for (const codec of CODECS) {
        for (const dir of ['encode', 'decode'] as Direction[]) {
          expect(() => apply(codec, dir, input)).not.toThrow();
        }
      }
    }
  });
});

describe('jwtView', () => {
  const token = `${b64url('{"alg":"HS256","typ":"JWT"}')}.${b64url('{"sub":"1","n":2}')}.c2ln`;

  it('jwt_view_valid', () => {
    const v = jwtView(enc(token))!;
    expect(v.header).toBe(JSON.stringify({ alg: 'HS256', typ: 'JWT' }, null, 2));
    expect(v.payload).toBe(JSON.stringify({ sub: '1', n: 2 }, null, 2));
    expect(v.signature).toBe('c2ln');
    expect(v.warnings).toEqual(['signature not verified']);
  });

  it('jwt_view_two_segments_null', () => {
    expect(jwtView(enc('a.b'))).toBeNull();
    expect(jwtView(enc('a.b.c.d'))).toBeNull();
  });

  it('jwt_view_non_json_payload_raw', () => {
    const t = `${b64url('{"alg":"HS256"}')}.${b64url('not json')}.x`;
    expect(jwtView(enc(t))!.payload).toBe('not json');
  });

  it('jwt_view_alg_none_warns', () => {
    const t = `${b64url('{"alg":"none"}')}.${b64url('{}')}.`;
    expect(jwtView(enc(t))!.warnings).toEqual(['signature not verified', 'alg is none']);
  });

  it('jwt_view_alg_missing_warns', () => {
    const t = `${b64url('{}')}.${b64url('{}')}.x`;
    expect(jwtView(enc(t))!.warnings).toEqual(['signature not verified', 'alg missing']);
  });

  it('jwt_view_alg_None_mixed_case_warns', () => {
    const t = `${b64url('{"alg":"None"}')}.${b64url('{}')}.`;
    expect(jwtView(enc(t))!.warnings).toEqual(['signature not verified', 'alg is none']);
  });
});

describe('html long entity', () => {
  it('html_long_unknown_entity_warns', () => {
    const r = apply('html', 'decode', enc('&verylongname;'));
    expect(new TextDecoder().decode(r.bytes)).toBe('&verylongname;');
    expect(r.warning).toBe('html: 1 unknown entities kept literally');
  });

  it('html_overlong_numeric_entity_warns', () => {
    for (const t of ['&#12345678901;', '&#x123456789;']) {
      const r = apply('html', 'decode', enc(t));
      expect(new TextDecoder().decode(r.bytes)).toBe(t);
      expect(r.warning).toBe('html: 1 unknown entities kept literally');
    }
  });
});
