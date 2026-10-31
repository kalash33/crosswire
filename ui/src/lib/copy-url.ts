// Client-side "Copy URL" for a recorded exchange.
//
// The history row (ExchangeRowDto) has NO tls flag and NO port: `host` is the
// request's Host header (lowercased; it carries `:port` when one was sent),
// falling back to the connection's target_addr. The TLS fact lives on the full
// exchange (`request.connection.tls_version`), which callers pass as `tls`
// when they have it, mirroring crosswire-tools' build_url. Without it the
// scheme is INFERRED: https when the port is 443, else http.
//
// The result is plain text for the clipboard: path and query are copied
// exactly as recorded, with no encoding changes.

export interface UrlSource {
  host: string | null;
  /** The request target as recorded. */
  path: string | null;
  /** From `request.connection.tls_version` when known; absent = infer. */
  tls?: boolean | null;
  /** From `request.connection.port` when known (0 = unknown). */
  port?: number | null;
}

/** Split a Host value into host and optional port; brackets an IPv6 literal. */
function splitHost(raw: string): { host: string; port: number | null } {
  const v6 = /^\[([^\]]*)\](?::(\d{1,5}))?$/.exec(raw);
  if (v6) return { host: `[${v6[1]}]`, port: v6[2] ? Number(v6[2]) : null };
  // More than one colon and no brackets: a bare IPv6 literal (target_addr).
  if (raw.indexOf(':') !== raw.lastIndexOf(':')) return { host: `[${raw}]`, port: null };
  const hp = /^(.*):(\d{1,5})$/.exec(raw);
  if (hp) return { host: hp[1], port: Number(hp[2]) };
  return { host: raw, port: null };
}

export function urlFor(src: UrlSource): string {
  const path = src.path ?? '';
  // An absolute-form request target already is the whole URL.
  if (/^[a-z][a-z0-9+.-]*:\/\//i.test(path)) return path;
  const { host, port: hostPort } = splitHost(src.host ?? '');
  const port = src.port && src.port > 0 ? src.port : hostPort;
  const tls = typeof src.tls === 'boolean' ? src.tls : port === 443;
  const scheme = tls ? 'https' : 'http';
  const def = tls ? 443 : 80;
  const authority = port !== null && port !== def ? `${host}:${port}` : host;
  return `${scheme}://${authority}${path === '' ? '/' : path}`;
}

/** The connection facts of `GET /exchanges/{id}`'s request, as urlFor wants them. */
export function connFacts(conn: { tls_version?: string | null; sni?: string | null; port?: number } | null | undefined): {
  tls: boolean | null;
  port: number | null;
} {
  if (!conn) return { tls: null, port: null };
  return {
    tls: typeof conn.tls_version === 'string' && conn.tls_version !== '',
    port: typeof conn.port === 'number' && conn.port > 0 ? conn.port : null,
  };
}
