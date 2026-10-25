import { describe, it, expect } from 'vitest';
import { hostRule, ruleFromRow, rowFromRule, parsePorts } from './scope-rows';

describe('scope rows', () => {
  it('hostRule: exact, host:port, ip', () => {
    expect(hostRule('a.test')).toEqual({ host: { type: 'exact', value: 'a.test' } });
    expect(hostRule('a.test:8443')).toEqual({ host: { type: 'exact', value: 'a.test' }, ports: [8443] });
    expect(hostRule('10.0.0.1')).toEqual({ host: { type: 'ip', value: '10.0.0.1' } });
  });
  it('round-trips a suffix rule with ports/scheme/path', () => {
    const r = {
      host: { type: 'suffix' as const, value: { suffix: 'x.test', also_apex: false } },
      ports: [80, 443],
      scheme: 'http' as const,
      path: { type: 'regex' as const, value: '^/a' },
    };
    expect(ruleFromRow(rowFromRule(r))).toEqual(r);
  });
  it('parsePorts rejects junk', () => {
    expect(() => parsePorts('80, x')).toThrow();
    expect(parsePorts('')).toBeUndefined();
  });
});
