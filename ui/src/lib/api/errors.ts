/** The stable server error envelope: `{ "error": { "code", "message" } }`. */
export class ApiError extends Error {
  readonly code: string;
  readonly status: number;

  constructor(code: string, message: string, status: number) {
    super(message);
    this.name = 'ApiError';
    this.code = code;
    this.status = status;
  }
}

/** HTTP 401: no valid session cookie. Callers re-bootstrap or show the sign-in state. */
export class UnauthorizedError extends ApiError {
  constructor(code: string, message: string) {
    super(code, message, 401);
    this.name = 'UnauthorizedError';
  }
}

/**
 * Build the right error for a non-2xx response from its (already read) text.
 * A body that is not the envelope still yields an ApiError: code `unknown`,
 * the real status, and the status text (or a generic line) as the message.
 */
export function errorFromResponse(status: number, statusText: string, text: string): ApiError {
  let code = 'unknown';
  let message = statusText || `request failed with status ${status}`;
  try {
    const parsed: unknown = JSON.parse(text);
    const env = (parsed as { error?: { code?: unknown; message?: unknown } } | null)?.error;
    if (env && typeof env.code === 'string' && typeof env.message === 'string') {
      code = env.code;
      message = env.message;
    }
  } catch {
    // Not JSON (proxy/HTML error page): keep the fallback.
  }
  return status === 401 ? new UnauthorizedError(code, message) : new ApiError(code, message, status);
}
