// HTTP/1 head tokenizer (U2 D1): the start line, then `Name: value` header
// lines (obs-fold continuations are values) up to the first empty line;
// anything after that is left uncoloured. A CR stays in the line (the editor
// keeps the bytes exact), so `\r` alone counts as the empty line.

import type { StreamParser } from '@codemirror/language';
import { streamLanguage, type Token } from './tokens';

interface State {
  line: number;
  body: boolean;
  /** Where the current header line is: name, after the colon, or value. */
  at: 'name' | 'colon' | 'value';
}

export const httpTokens: StreamParser<State> = {
  name: 'http',
  startState: () => ({ line: 0, body: false, at: 'name' }),
  copyState: (s) => ({ ...s }),
  blankLine(state) {
    if (state.line > 0) state.body = true;
    state.line++;
  },
  token(stream, state): Token | null {
    if (stream.sol()) {
      state.line++;
      // obs-fold: a line starting with blanks continues the previous value.
      state.at = /^[ \t]/.test(stream.string) ? 'value' : 'name';
      if (state.line > 1 && !state.body && stream.string === '\r') state.body = true;
    }
    if (state.body) {
      stream.skipToEnd();
      return null;
    }
    if (state.line === 1) {
      stream.skipToEnd();
      return 'statusLine';
    }
    if (state.at === 'name') {
      state.at = 'colon';
      if (stream.match(/^[^:]+/)) return 'headerName';
    }
    if (state.at === 'colon') {
      state.at = 'value';
      if (stream.eat(':')) return 'punctuation';
    }
    if (stream.eatSpace()) return null;
    stream.skipToEnd();
    return 'headerValue';
  },
};

export const httpLanguage = streamLanguage(httpTokens, 'http');
