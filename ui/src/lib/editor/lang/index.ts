// The viewer's colouring languages (U2 D1): a language extension plus the
// class-only colouring plugin, or nothing.

import type { Extension } from '@codemirror/state';
import { jsonLanguage } from './json';
import { httpLanguage } from './http';
import { markupLanguage } from './markup';
import { tokenClasses } from './tokens';

export type EditorLanguage = 'json' | 'http' | 'markup';

const LANGS = { json: jsonLanguage, http: httpLanguage, markup: markupLanguage } as const;

export function languageExtension(lang: EditorLanguage | null | undefined): Extension {
  return lang ? [LANGS[lang], tokenClasses] : [];
}
