// What the UI knows about the server it talks to: the API origin (its own
// origin), and `GET /api/v1/health`. Health has no proxy address today; a
// future `proxy_addr` (and app `version`) field is picked up when present,
// validated as a plain host:port before it reaches a URL or a command.

import { apiGet } from '../api/client';
import type { HealthResponse } from '../api/types';
import { DEFAULT_PROXY_ADDR, parseProxyAddr } from '../components/start';

export interface HealthInfo {
  schemaVersion: number;
  formatVersion: number;
  /** App version, when the server reports one. */
  version: string | null;
}

type HealthWire = HealthResponse & { proxy_addr?: unknown; version?: unknown };

function createConnection() {
  let health = $state<HealthInfo | null>(null);
  let proxyAddr = $state<string | null>(null);
  let error = $state<string | null>(null);
  let inflight: Promise<void> | null = null;

  async function fetchHealth(): Promise<void> {
    try {
      const h = await apiGet<HealthWire>('/api/v1/health');
      health = {
        schemaVersion: h.schema_version,
        formatVersion: h.format_version,
        version: typeof h.version === 'string' && h.version.length <= 64 ? h.version : null,
      };
      proxyAddr = parseProxyAddr(h.proxy_addr);
      error = null;
    } catch (e) {
      error = e instanceof Error ? e.message : 'health check failed';
    }
  }

  return {
    get health(): HealthInfo | null {
      return health;
    },
    /** The proxy address the server reported, or null when it did not. */
    get proxyAddr(): string | null {
      return proxyAddr;
    },
    /** The reported address, else the `crosswire serve` default. */
    get proxyAddrOrDefault(): string {
      return proxyAddr ?? DEFAULT_PROXY_ADDR;
    },
    get error(): string | null {
      return error;
    },
    get apiOrigin(): string {
      return location.origin;
    },
    /** Fetch health once per page load (concurrent callers share it). */
    load(): Promise<void> {
      inflight ??= fetchHealth();
      return inflight;
    },
  };
}

export const connection = createConnection();
