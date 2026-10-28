// Preview wiring: mint a per-exchange capability token and build the iframe
// src on the isolated 127.0.0.1 origin (never the UI's localhost origin).
import { apiGet } from './client';

export interface PreviewToken {
  token: string;
  path: string;
  expires_in?: number;
}

export function previewToken(id: string): Promise<PreviewToken> {
  return apiGet<PreviewToken>(`/api/v1/exchanges/${id}/preview-token`);
}

/** `http://127.0.0.1:<this port>` + the minted path: a different browser origin. */
export function previewSrc(path: string, port: string = window.location.port): string {
  if (!path.startsWith('/preview/')) {
    throw new Error('unexpected preview path');
  }
  return `http://127.0.0.1:${port}${path}`;
}
