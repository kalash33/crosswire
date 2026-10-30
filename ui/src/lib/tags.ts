// Client-side tag validation. The store (`exchange_set_tags`) enforces no
// bounds of its own, so these limits are a UI guard; the server stays the
// authority on what it accepts.

export const MAX_TAG_LEN = 64;
export const MAX_TAGS = 32;

/** Matches `/[\u0000-\u001f\u007f]/` without a control-char regex literal. */
function hasControl(s: string): boolean {
  for (let i = 0; i < s.length; i++) {
    const c = s.charCodeAt(i);
    if (c <= 0x1f || c === 0x7f) return true;
  }
  return false;
}

export type AddTagResult = { ok: true; tags: string[] } | { ok: false; error: string };

/** Trim `raw` and append it to `tags`, or say why it cannot be added. */
export function addTag(tags: readonly string[], raw: string): AddTagResult {
  const tag = raw.trim();
  if (tag === '') return { ok: false, error: 'tag is empty' };
  if (tag.length > MAX_TAG_LEN) return { ok: false, error: `tag is longer than ${MAX_TAG_LEN} characters` };
  if (hasControl(tag)) return { ok: false, error: 'tag contains control characters' };
  if (tags.includes(tag)) return { ok: false, error: 'tag already present' };
  if (tags.length >= MAX_TAGS) return { ok: false, error: `at most ${MAX_TAGS} tags` };
  return { ok: true, tags: [...tags, tag] };
}
