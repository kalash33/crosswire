import { bytesToLatin1, encodePastedText, latin1ToBytes } from '../editor/encoding';

// A native <textarea> normalizes CR/CRLF to LF in `.value`, which would silently
// drop 0x0D bytes. So CR is shown as a visible sentinel (U+240D), the same
// convention the editor uses for non-printables.
const CR = '\r';
const SENTINEL = '␍';

export function toTextareaText(bytes: Uint8Array): string {
  return bytesToLatin1(bytes).replaceAll(CR, SENTINEL);
}

/** A literal typed U+240D is indistinguishable from the sentinel and becomes a CR byte. */
export function fromTextareaText(text: string): { bytes: Uint8Array; nonLatin1: boolean } {
  const s = text.replaceAll(SENTINEL, CR);
  for (let i = 0; i < s.length; i++) {
    if (s.charCodeAt(i) > 0xff) return encodePastedText(s);
  }
  return { bytes: latin1ToBytes(s), nonLatin1: false };
}
