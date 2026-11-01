// What the Pretty tab can show for one message (U2 D1). Everything here is
// derived in the UI from the recorded bytes; nothing is sent anywhere.
//
//  * json  - `application/json` / `+json` (reason 'type'), or a body that
//            parses as a JSON object or array (reason 'parse');
//  * form  - `application/x-www-form-urlencoded`;
//  * query - a request target with `?` (a key/value table above the body);
//  * none  - nothing to prettify: the viewer opens on Raw.
//
// Bodies over PRETTY_LIMIT are never parsed: a typed one reports
// `tooLarge` (Pretty shows a note), an untyped one is simply 'none'.

import { parseHeadLines } from '../http/headers';
import { decodeUtf8 } from './pretty';

/** 2 MiB: the largest body the Pretty tab parses. */
export const PRETTY_LIMIT = 2 * 1024 * 1024;

export type PrettyKind = 'json' | 'form' | 'query' | 'none';

export interface Detection {
  kind: PrettyKind;
  reason: 'type' | 'parse' | 'none';
  tooLarge: boolean;
}

export type ViewerTab = 'pretty' | 'raw' | 'headers' | 'hex';

/** The media type of a raw HTTP/1 head, lowercased, without parameters ('' when absent). */
export function contentTypeOf(head: Uint8Array | null): string {
  if (!head) return '';
  const h = parseHeadLines(head).headers.find((x) => x.name.trim().toLowerCase() === 'content-type');
  return h ? mediaType(h.value) : '';
}

/** `Application/JSON; charset=utf-8` -> `application/json`. */
export function mediaType(value: string): string {
  return value.split(';')[0].trim().toLowerCase();
}

export function isJsonType(type: string): boolean {
  return type === 'application/json' || type.endsWith('+json');
}

export function isMarkupType(type: string): boolean {
  return (
    type === 'text/html' ||
    type === 'application/xhtml+xml' ||
    type === 'text/xml' ||
    type === 'application/xml' ||
    type === 'image/svg+xml' ||
    type.endsWith('+xml')
  );
}

/** True when `body` is a JSON object or array (UTF-8 decoded). */
export function parsesAsJson(body: Uint8Array): boolean {
  if (body.length === 0 || body.length > PRETTY_LIMIT) return false;
  // Cheap reject before decoding: the first non-blank byte must open a container.
  let i = 0;
  while (i < body.length && (body[i] === 0x20 || body[i] === 0x09 || body[i] === 0x0a || body[i] === 0x0d)) i++;
  // A UTF-8 BOM is allowed in front (TextDecoder drops it).
  if (body[i] === 0xef && body[i + 1] === 0xbb && body[i + 2] === 0xbf) i += 3;
  if (body[i] !== 0x7b && body[i] !== 0x5b) return false;
  try {
    JSON.parse(decodeUtf8(body));
    return true;
  } catch {
    return false;
  }
}

export function detectPretty(
  head: Uint8Array | null,
  body: Uint8Array,
  contentType?: string,
  query?: string,
): Detection {
  const type = contentType !== undefined && contentType !== '' ? mediaType(contentType) : contentTypeOf(head);
  const tooLarge = body.length > PRETTY_LIMIT;
  if (isJsonType(type)) return { kind: 'json', reason: 'type', tooLarge };
  if (type === 'application/x-www-form-urlencoded') return { kind: 'form', reason: 'type', tooLarge };
  if (parsesAsJson(body)) return { kind: 'json', reason: 'parse', tooLarge: false };
  if (query !== undefined && query.includes('?')) return { kind: 'query', reason: 'type', tooLarge: false };
  return { kind: 'none', reason: 'none', tooLarge: false };
}

/**
 * The tab a message opens on. Pretty when there is something to prettify,
 * except that JSON found only by parsing keeps Raw when the message declares
 * another real type (a JSON-looking `text/html` body is shown as recorded).
 */
export function defaultTab(d: Detection, contentType: string): ViewerTab {
  if (d.kind === 'none' || d.tooLarge) return 'raw';
  if (d.reason === 'parse') {
    const t = mediaType(contentType);
    return t === '' || t === 'text/plain' ? 'pretty' : 'raw';
  }
  return 'pretty';
}
