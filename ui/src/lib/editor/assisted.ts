// Assisted-mode chip state — DISPLAY only.
//
// In assisted send mode the SERVER applies the real edits to the outgoing
// request (recompute Content-Length, normalize line endings). These helpers only
// describe, ahead of time, what that will look like, and afterwards label what
// actually happened. They NEVER mutate bytes.
//
// The one piece of real logic here is the Transfer-Encoding guard: per RFC 7230,
// a message that carries `Transfer-Encoding` must NOT also be framed by
// `Content-Length` (and a TE message is framed by the chunked encoding, not CL).
// So when TE is present we must never show a "will rewrite Content-Length" chip —
// doing so would misrepresent what the server does and could mask a
// request-smuggling-relevant framing decision. We surface "skipped: TE present"
// instead.
//
// All byte<->string conversion routes through encoding.ts (Latin-1), never a
// WHATWG TextDecoder — the C1 range (0x80..0x9F) must survive the head scan.

import { bytesToLatin1 } from './encoding';
import type { ChangeDto } from '../api/types';
import type { Chip } from './chips';

export type { Chip } from './chips';

/** The assisted-mode toggles the user controls (both default on in the UI). */
export interface AssistedToggles {
  /** Recompute Content-Length on send (ignored on a TE message). */
  contentLength: boolean;
  /** Normalize bare LF / lone CR to CRLF in the head on send. */
  lineEndings: boolean;
}

// Does the request HEAD carry a Transfer-Encoding header? We scan only the head
// (up to the first blank line) and compare header names case-insensitively. A
// folded/obs-fold continuation can never introduce a new header name, so a
// simple line scan is sufficient and conservative.
function hasTransferEncoding(raw: Uint8Array): boolean {
  const text = bytesToLatin1(raw);
  // Head ends at the first empty line (CRLF CRLF or LF LF). Normalize the search
  // by splitting on LF and stripping a trailing CR per line.
  const lines = text.split('\n');
  for (let i = 0; i < lines.length; i++) {
    let line = lines[i];
    if (line.endsWith('\r')) line = line.slice(0, -1);
    // Blank line => end of head; headers after this are body, not header fields.
    if (i > 0 && line === '') break;
    // The request/status line (i === 0) has no field name.
    if (i === 0) continue;
    const colon = line.indexOf(':');
    if (colon <= 0) continue;
    const name = line.slice(0, colon).trim().toLowerCase();
    if (name === 'transfer-encoding') return true;
  }
  return false;
}

/**
 * The assisted-mode chips to show ABOVE the request editor, describing what the
 * server will do on send. Display only; the server performs the real edits.
 */
export function assistedChips(raw: Uint8Array, toggles: AssistedToggles): Chip[] {
  const chips: Chip[] = [];

  // Content-Length. TE wins unconditionally: never imply a CL rewrite on a TE
  // message, even if the toggle is on.
  if (hasTransferEncoding(raw)) {
    chips.push({ label: 'Content-Length', state: 'skipped: TE present' });
  } else if (toggles.contentLength) {
    chips.push({ label: 'Content-Length', state: 'update' });
  } else {
    chips.push({ label: 'Content-Length', state: 'off' });
  }

  // Line endings. Reflects the normalization intent only.
  chips.push({ label: 'Line endings', state: toggles.lineEndings ? 'normalize CRLF' : 'preserve' });

  return chips;
}

/**
 * Turn a `SendOutcomeDto.changes` list (what the server actually changed) into
 * labeled result chips for display after a send.
 */
export function renderChanges(changes: ChangeDto[]): Chip[] {
  return changes.map((c) => ({ label: `${c.op} @ ${c.start}‥${c.end}`, state: c.reason }));
}
