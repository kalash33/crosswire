// JSON tokenizer (U2 D1). Line-based and forgiving: it colours what it can
// recognise and steps over anything else one character at a time, so hostile
// or broken input never throws and always advances.

import type { StreamParser } from '@codemirror/language';
import { streamLanguage, type Token } from './tokens';

type State = Record<string, never>;

export const jsonTokens: StreamParser<State> = {
  name: 'json',
  startState: () => ({}),
  token(stream): Token | null {
    if (stream.eatSpace()) return null;
    const ch = stream.peek();
    if (ch === '"') {
      stream.match(/^"(?:[^"\\]|\\.?)*"?/);
      // A string followed by `:` is an object key.
      return /^\s*:/.test(stream.string.slice(stream.pos)) ? 'propertyName' : 'string';
    }
    if (stream.match(/^-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?/)) return 'number';
    if (stream.match(/^(?:true|false|null)\b/)) return 'keyword';
    if (ch !== undefined && '{}[],:'.includes(ch)) {
      stream.next();
      return 'punctuation';
    }
    stream.next();
    return null;
  },
};

export const jsonLanguage = streamLanguage(jsonTokens, 'json');
