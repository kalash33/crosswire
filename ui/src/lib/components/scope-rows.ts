// Editor-row model for scope rules: flat strings the form binds to, converted
// to/from the wire `Rule`. Everything is plain text (§11.1).
import type { HostPattern, PathMatch, Rule, Scheme, Scope } from '../api/scope';

export interface RuleRow {
  hostType: HostPattern['type'];
  /** exact/regex/ip/cidr value, or the suffix for `suffix`. */
  hostValue: string;
  alsoApex: boolean;
  portsText: string;
  scheme: '' | Scheme;
  pathType: '' | PathMatch['type'];
  pathValue: string;
}

export function blankRow(): RuleRow {
  return { hostType: 'exact', hostValue: '', alsoApex: true, portsText: '', scheme: '', pathType: '', pathValue: '' };
}

export function rowFromRule(r: Rule): RuleRow {
  const h = r.host;
  return {
    hostType: h.type,
    hostValue: h.type === 'suffix' ? h.value.suffix : h.value,
    alsoApex: h.type === 'suffix' ? h.value.also_apex : true,
    portsText: (r.ports ?? []).join(', '),
    scheme: r.scheme ?? '',
    pathType: r.path?.type ?? '',
    pathValue: r.path?.value ?? '',
  };
}

export class RowError extends Error {}

export function parsePorts(text: string): number[] | undefined {
  const toks = text.split(/[\s,]+/).filter((t) => t !== '');
  if (toks.length === 0) return undefined;
  const ports = toks.map((t) => {
    const n = Number(t);
    if (!/^\d+$/.test(t) || n < 0 || n > 65535) throw new RowError(`invalid port: ${t}`);
    return n;
  });
  return [...new Set(ports)].sort((a, b) => a - b);
}

export function ruleFromRow(row: RuleRow): Rule {
  const v = row.hostValue.trim();
  if (v === '') throw new RowError('a rule has an empty host');
  let host: HostPattern;
  switch (row.hostType) {
    case 'suffix':
      host = { type: 'suffix', value: { suffix: v, also_apex: row.alsoApex } };
      break;
    case 'exact':
    case 'regex':
    case 'ip':
    case 'cidr':
      host = { type: row.hostType, value: v };
      break;
  }
  const rule: Rule = { host };
  const ports = parsePorts(row.portsText);
  if (ports) rule.ports = ports;
  if (row.scheme) rule.scheme = row.scheme;
  if (row.pathType) rule.path = { type: row.pathType, value: row.pathValue };
  return rule;
}

export interface Draft {
  include: RuleRow[];
  exclude: RuleRow[];
}

export const draftFromScope = (s: Scope): Draft => ({
  include: s.include.map(rowFromRule),
  exclude: s.exclude.map(rowFromRule),
});

export const scopeFromDraft = (d: Draft): Scope => ({
  include: d.include.map(ruleFromRow),
  exclude: d.exclude.map(ruleFromRow),
});

/** One-line text summary of a rule, for previews. */
export function describeRule(r: Rule): string {
  const h = r.host;
  const host =
    h.type === 'suffix' ? `suffix ${h.value.suffix}${h.value.also_apex ? ' (+apex)' : ''}` : `${h.type} ${h.value}`;
  const parts = [host];
  if (r.ports?.length) parts.push(`ports ${r.ports.join(',')}`);
  if (r.scheme) parts.push(r.scheme);
  if (r.path) parts.push(`path ${r.path.type} ${r.path.value}`);
  return parts.join(' · ');
}

/**
 * An include rule for a captured host. A trailing `:port` becomes a port
 * constraint; an IPv4/IPv6 literal becomes an `ip` pattern, else `exact`.
 */
export function hostRule(rawHost: string): Rule {
  let host = rawHost.trim();
  let ports: number[] | undefined;
  const m = /^([^:[\]]+):(\d{1,5})$/.exec(host);
  if (m) {
    host = m[1];
    ports = [Number(m[2])];
  }
  const isIp = /^\d{1,3}(\.\d{1,3}){3}$/.test(host) || (host.includes(':') && /^[0-9a-fA-F:]+$/.test(host));
  const rule: Rule = { host: { type: isIp ? 'ip' : 'exact', value: host } };
  if (ports) rule.ports = ports;
  return rule;
}
