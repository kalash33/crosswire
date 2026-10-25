import { apiGet, apiSend } from '../api/client';
import { ApiError, UnauthorizedError } from '../api/errors';
import type { SessionRequest } from '../api/types';

export type SessionStatus = 'unknown' | 'authenticated' | 'unauthenticated';

const FRAGMENT_KEY = 'bootstrap';

/** Pull the bootstrap token out of a fragment, a full URL, or a bare token. */
function extractToken(raw: string): string {
  const text = raw.trim();
  const m = new RegExp(`[#&?]${FRAGMENT_KEY}=([^&\\s]+)`).exec(text);
  return m ? decodeURIComponent(m[1]) : text;
}

function fragmentToken(): string | null {
  const hash = location.hash.startsWith('#') ? location.hash.slice(1) : location.hash;
  const t = new URLSearchParams(hash).get(FRAGMENT_KEY);
  return t ? t : null;
}

function describe(e: unknown): string {
  return e instanceof Error ? e.message : 'request failed';
}

/**
 * Session state. The bootstrap token is single-use and only ever lives in a
 * local variable for the duration of the exchange request; the resulting
 * credential is an HttpOnly cookie that JS cannot read. Nothing is persisted
 * (no localStorage / sessionStorage).
 */
class Session {
  status = $state<SessionStatus>('unknown');
  error = $state<string | null>(null);

  private async exchange(token: string): Promise<void> {
    this.error = null;
    try {
      const body: SessionRequest = { bootstrap: token };
      await apiSend('POST', '/api/v1/session', body);
      this.status = 'authenticated';
    } catch (e) {
      this.status = 'unauthenticated';
      this.error = describe(e);
    }
  }

  /** Exchange a `#bootstrap=<token>` fragment for the session cookie, if present. */
  async bootstrapFromFragment(): Promise<void> {
    const token = fragmentToken();
    if (token === null) return;
    // Drop the token from the URL before the request: it is single-use, so it
    // must not linger in the address bar/history whether or not this succeeds.
    history.replaceState(null, '', location.pathname + location.search);
    await this.exchange(token);
  }

  /** Paste path: accepts a bare token or a full bootstrap URL. */
  async submitToken(raw: string): Promise<void> {
    const token = extractToken(raw);
    if (token === '') {
      this.status = 'unauthenticated';
      this.error = 'Paste the bootstrap URL or token printed by the server.';
      return;
    }
    await this.exchange(token);
  }

  /**
   * Cheap authenticated GET. 200 -> authenticated; 401 -> unauthenticated.
   * Any other failure (network down, 5xx) cannot prove a session either way:
   * if we were still `unknown` it falls back to `unauthenticated` so the auth
   * screen renders (never an endless spinner); an already-`authenticated`
   * state is left alone so a transient error does not log the user out.
   */
  async probe(): Promise<void> {
    try {
      await apiGet('/api/v1/counts');
      this.status = 'authenticated';
    } catch (e) {
      if (e instanceof UnauthorizedError) {
        this.status = 'unauthenticated';
      } else if (this.status !== 'authenticated') {
        this.status = 'unauthenticated';
        if (e instanceof ApiError || e instanceof Error) this.error = describe(e);
      }
    }
  }
}

export const session = new Session();
