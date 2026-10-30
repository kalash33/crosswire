import { apiSend } from './client';
import type { PatchBody } from './types';
import type { ExchangeDto } from '../stores/history.svelte';

/**
 * `PATCH /exchanges/{id}` — REPLACE semantics: a present `tags` is the whole
 * new tag list, a present `notes` is the whole new note. Returns the merged
 * exchange.
 */
export function patchExchange(id: string, body: PatchBody): Promise<ExchangeDto> {
  return apiSend<ExchangeDto>('PATCH', `/api/v1/exchanges/${encodeURIComponent(id)}`, body);
}
