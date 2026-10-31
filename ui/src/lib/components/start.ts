// Pure helpers for the Start panel and the Connection card (unit-tested).

/** `crosswire serve --proxy-bind` default; shown when the server does not report it. */
export const DEFAULT_PROXY_ADDR = '127.0.0.1:8080';

/** The Start panel replaces History only for an empty, unfiltered, loaded list. */
export function showStart(rows: number, filterActive: boolean, loading: boolean): boolean {
  return rows === 0 && !filterActive && !loading;
}

/**
 * A server-reported proxy address, accepted only as a plain `host:port`
 * (IPv4, hostname or bracketed IPv6) so it can never smuggle a scheme, path
 * or whitespace into the CA page URL or the copied command.
 */
export function parseProxyAddr(v: unknown): string | null {
  if (typeof v !== 'string') return null;
  const m = /^(\[[0-9a-fA-F:.]+\]|[A-Za-z0-9.-]+):(\d{1,5})$/.exec(v);
  if (!m) return null;
  const port = Number(m[2]);
  return port > 0 && port <= 65535 ? v : null;
}

/** The proxy's own info page (it serves `/`, `/ca.der`, `/ca.pem` directly). */
export function caPageUrl(proxyAddr: string): string {
  return `http://${proxyAddr}/`;
}

export function browserCommand(proxyAddr: string, projectDir?: string): string {
  return `crosswire browser --project ${projectDir ?? '<project>'} --proxy ${proxyAddr}`;
}
