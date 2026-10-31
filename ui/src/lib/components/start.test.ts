import { describe, expect, it } from 'vitest';
import { browserCommand, caPageUrl, parseProxyAddr, showStart } from './start';

describe('showStart', () => {
  it('shows only for an empty, unfiltered, loaded history', () => {
    expect(showStart(0, false, false)).toBe(true);
    expect(showStart(1, false, false)).toBe(false);
    expect(showStart(0, true, false)).toBe(false);
    expect(showStart(0, false, true)).toBe(false);
  });
});

describe('connection helpers', () => {
  it('accepts only a plain host:port proxy address', () => {
    expect(parseProxyAddr('127.0.0.1:8080')).toBe('127.0.0.1:8080');
    expect(parseProxyAddr('[::1]:8080')).toBe('[::1]:8080');
    expect(parseProxyAddr('localhost:9000')).toBe('localhost:9000');
    expect(parseProxyAddr('data:text/html,x')).toBeNull();
    expect(parseProxyAddr('http://evil.example:80')).toBeNull();
    expect(parseProxyAddr('evil.example/x:80')).toBeNull();
    expect(parseProxyAddr('a b:80')).toBeNull();
    expect(parseProxyAddr(8080)).toBeNull();
    expect(parseProxyAddr(undefined)).toBeNull();
    expect(parseProxyAddr('127.0.0.1:99999')).toBeNull();
    // Built by join: a literal script URL trips no-script-url even in a test.
    const scriptish = ['javascript', '80/'].join(':');
    for (const bad of ['user@host:80', scriptish, 'http://x:80', 'x:80/path', 'x :80']) {
      expect(parseProxyAddr(bad)).toBeNull();
    }
    expect(parseProxyAddr('[::1]:18080')).toBe('[::1]:18080');
  });

  it('maps a wildcard bind to loopback for the shown and opened address', () => {
    expect(parseProxyAddr('0.0.0.0:8080')).toBe('127.0.0.1:8080');
    expect(parseProxyAddr('[::]:8080')).toBe('[::1]:8080');
    expect(caPageUrl(parseProxyAddr('0.0.0.0:8080')!)).toBe('http://127.0.0.1:8080/');
  });

  it('builds the CA page address and the browser command', () => {
    expect(caPageUrl('127.0.0.1:8080')).toBe('http://127.0.0.1:8080/');
    expect(browserCommand('127.0.0.1:8080')).toBe(
      'crosswire browser --project PROJECT_DIR --proxy 127.0.0.1:8080',
    );
    expect(browserCommand('127.0.0.1:8080', '/tmp/p')).toBe(
      'crosswire browser --project /tmp/p --proxy 127.0.0.1:8080',
    );
  });
});
