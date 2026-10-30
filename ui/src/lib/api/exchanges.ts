import { apiSend } from './client';
import type { PatchBody } from './types';

/**
 * `PATCH /exchanges/{id}` — REPLACE semantics: a present `tags` is the whole
 * new tag list, a present `notes` is the whole new note. The real route answers 200 with an
 * empty body, so this resolves with nothing.
 */
export function patchExchange(id: string, body: PatchBody): Promise<void> {
  return apiSend<void>('PATCH', `/api/v1/exchanges/${encodeURIComponent(id)}`, body);
}
