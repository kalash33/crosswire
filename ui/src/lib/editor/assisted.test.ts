import { describe, it, expect } from 'vitest';
import { assistedChips, renderChanges } from './assisted';
import type { ChangeDto } from '../api/types';

// Build request-head bytes from an ASCII head string. Heads are ASCII on the
// wire, so UTF-8 encoding equals the raw bytes here; the production detection
// path decodes through encoding.ts (Latin-1), which agrees for ASCII.
const head = (s: string): Uint8Array => new TextEncoder().encode(s);

const CL_HEAD = head(['POST /x HTTP/1.1', 'Host: a', 'Content-Length: 5', '', 'hello'].join('\r\n'));
const TE_HEAD = head(
  ['POST /x HTTP/1.1', 'Host: a', 'Transfer-Encoding: chunked', '', '0', '', ''].join('\r\n'),
);
// A message that carries BOTH headers: TE must win; CL must never be "update".
const TE_AND_CL_HEAD = head(
  ['POST /x HTTP/1.1', 'Transfer-Encoding: chunked', 'Content-Length: 5', '', ''].join('\r\n'),
);

function chip(chips: { label: string; state?: string }[], label: string) {
  return chips.find((c) => c.label === label);
}

describe('assistedChips', () => {
  it('emits the CL chip as "skipped: TE present" when Transfer-Encoding is present', () => {
    const chips = assistedChips(TE_HEAD, { contentLength: true, lineEndings: false });
    expect(chip(chips, 'Content-Length')?.state).toBe('skipped: TE present');
  });

  it('never rewrites CL on a TE message even with both headers and the toggle on', () => {
    const chips = assistedChips(TE_AND_CL_HEAD, { contentLength: true, lineEndings: false });
    expect(chip(chips, 'Content-Length')?.state).toBe('skipped: TE present');
  });

  it('detects Transfer-Encoding case-insensitively', () => {
    const lower = head(['POST / HTTP/1.1', 'transfer-encoding: chunked', '', ''].join('\r\n'));
    expect(chip(assistedChips(lower, { contentLength: true, lineEndings: false }), 'Content-Length')?.state).toBe(
      'skipped: TE present',
    );
  });

  it('emits a CL "update" chip when Content-Length is present and the toggle is on', () => {
    const chips = assistedChips(CL_HEAD, { contentLength: true, lineEndings: false });
    expect(chip(chips, 'Content-Length')?.state).toBe('update');
  });

  it('emits a CL "off" chip when the toggle is off', () => {
    const chips = assistedChips(CL_HEAD, { contentLength: false, lineEndings: false });
    expect(chip(chips, 'Content-Length')?.state).toBe('off');
  });

  it('reflects the line-ending normalization toggle', () => {
    const on = assistedChips(CL_HEAD, { contentLength: false, lineEndings: true });
    const off = assistedChips(CL_HEAD, { contentLength: false, lineEndings: false });
    expect(chip(on, 'Line endings')?.state).toBe('normalize CRLF');
    expect(chip(off, 'Line endings')?.state).toBe('preserve');
  });
});

describe('renderChanges', () => {
  it('maps a SendOutcomeDto.changes list to labeled result chips', () => {
    const chips = renderChanges([
      { op: 'replace', reason: 'hop-by-hop', start: 20, end: 35 },
      { op: 'replace', reason: { 'intruder-position': 3 }, start: 40, end: 44 },
    ]);
    expect(chips).toEqual([
      { label: 'replace @ 20‥35', state: 'hop-by-hop' },
      { label: 'replace @ 40‥44', state: 'intruder-position' },
    ]);
  });

  it('falls back to unknown for a null, array or empty reason', () => {
    const bad = [null, [], {}] as unknown as ChangeDto['reason'][];
    const chips = renderChanges(bad.map((reason) => ({ op: 'replace', reason, start: 0, end: 1 })));
    expect(chips.map((c) => c.state)).toEqual(['unknown', 'unknown', 'unknown']);
  });

  it('returns an empty list for no changes', () => {
    expect(renderChanges([])).toEqual([]);
  });
});
