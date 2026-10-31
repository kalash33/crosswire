import { describe, expect, it } from 'vitest';
import { urlFor } from './copy-url';

describe('urlFor', () => {
  it('uses the TLS flag for the scheme and elides default ports', () => {
    expect(urlFor({ host: 'a.test', path: '/x?y=1', tls: true, port: 443 })).toBe('https://a.test/x?y=1');
    expect(urlFor({ host: 'a.test', path: '/x', tls: false, port: 80 })).toBe('http://a.test/x');
    expect(urlFor({ host: 'a.test', path: '/', tls: true, port: 80 })).toBe('https://a.test:80/');
    expect(urlFor({ host: 'a.test', path: '/', tls: false, port: 8080 })).toBe('http://a.test:8080/');
  });

  it('keeps the path and query exactly as recorded', () => {
    expect(urlFor({ host: 'a.test', path: '/a%20b/c?q=<x>&r=%zz#f', tls: false })).toBe(
      'http://a.test/a%20b/c?q=<x>&r=%zz#f',
    );
  });

  it('reads a port from the Host value and infers https from 443 without a TLS flag', () => {
    expect(urlFor({ host: 'a.test:443', path: '/' })).toBe('https://a.test/');
    expect(urlFor({ host: 'a.test:8443', path: '/' })).toBe('http://a.test:8443/');
    expect(urlFor({ host: 'a.test', path: '/' })).toBe('http://a.test/');
    expect(urlFor({ host: 'a.test:8443', path: '/', tls: true })).toBe('https://a.test:8443/');
  });

  it('brackets an IPv6 host and handles a bracketed host with a port', () => {
    expect(urlFor({ host: '::1', path: '/', tls: false, port: 8080 })).toBe('http://[::1]:8080/');
    expect(urlFor({ host: '[2001:db8::1]:443', path: '/p' })).toBe('https://[2001:db8::1]/p');
    expect(urlFor({ host: '[::1]', path: '/', tls: true })).toBe('https://[::1]/');
  });

  it('returns an absolute-form request target unchanged and copes with nulls', () => {
    expect(urlFor({ host: 'a.test', path: 'http://b.test/x' })).toBe('http://b.test/x');
    expect(urlFor({ host: null, path: null })).toBe('http:///');
  });
});
