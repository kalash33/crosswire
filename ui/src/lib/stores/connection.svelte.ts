// What the UI knows about the server it talks to: the API origin (its own
// origin), and `GET /api/v1/health`. The optional `proxy_addr` (and a future
// app `version`) is picked up when present, validated as a plain host:port
// before it reaches a URL or a command. The optional `project` (name and path)
// is display text only: shape- and length-checked, then rendered as text.

import { apiGet } from '../api/client';
import type { HealthResponse, ProjectInfo } from '../api/types';
import { DEFAULT_PROXY_ADDR, parseProxyAddr } from '../components/start';

export interface HealthInfo {
  schemaVersion: number;
  formatVersion: number;
  /** App version, when the server reports one. */
  version: string | null;
}

type HealthWire = Omit<HealthResponse, 'project'> & {
  proxy_addr?: unknown;
  version?: unknown;
  project?: unknown;
};

/** Longest project name and path the UI shows; anything longer is dropped. */
const MAX_NAME = 255;
const MAX_PATH = 4096;

/** The reported project, or null when absent or malformed. */
export function parseProject(v: unknown): ProjectInfo | null {
  if (typeof v !== 'object' || v === null) return null;
  const { name, path } = v as { name?: unknown; path?: unknown };
  if (typeof name !== 'string' || typeof path !== 'string') return null;
  if (name.trim() === '' || name.length > MAX_NAME || path.length > MAX_PATH) return null;
  return { name, path };
}

export function createConnection() {
  let health = $state<HealthInfo | null>(null);
  let proxyAddr = $state<string | null>(null);
  let project = $state<ProjectInfo | null>(null);
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
      project = parseProject(h.project);
      error = null;
    } catch (e) {
      error = e instanceof Error ? e.message : 'health check failed';
      inflight = null; // let the next caller retry
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
    /** The project the server records into, or null when it names none. */
    get project(): ProjectInfo | null {
      return project;
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
