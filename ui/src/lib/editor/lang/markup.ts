// Minimal HTML/XML tokenizer (U2 D1): tags, attribute names and values, and
// comments, with quoted values and comments allowed to span lines. Text
// content is left uncoloured. It only ever classifies characters; nothing it
// sees is interpreted.

import type { StreamParser } from '@codemirror/language';
import { streamLanguage, type Token } from './tokens';

interface State {
  /** Inside a comment that has not closed yet. */
  comment: boolean;
  /** Inside a tag. */
  tag: boolean;
  /** Right after `<` or `</`: the next run is the tag name. */
  name: boolean;
  /** After `=` inside a tag: the next run is a value. */
  value: boolean;
  /** The quote of an open, multi-line attribute value. */
  quote: string | null;
}

export const markupTokens: StreamParser<State> = {
  name: 'markup',
  startState: () => ({ comment: false, tag: false, name: false, value: false, quote: null }),
  copyState: (s) => ({ ...s }),
  token(stream, state): Token | null {
    if (state.comment) {
      if (stream.skipTo('-->')) {
        stream.pos += 3;
        state.comment = false;
      } else {
        stream.skipToEnd();
      }
      return 'comment';
    }
    if (state.quote !== null) {
      const q = state.quote;
      const end = stream.string.indexOf(q, stream.pos);
      if (end < 0) {
        stream.skipToEnd();
      } else {
        stream.pos = end + 1;
        state.quote = null;
      }
      return 'attributeValue';
    }
    if (state.tag) {
      if (state.name) {
        state.name = false;
        if (stream.match(/^[^\s=>/"']+/)) return 'tagName';
      }
      if (stream.eatSpace()) return null;
      if (stream.match('/>') || stream.eat('>')) {
        state.tag = false;
        state.value = false;
        return 'punctuation';
      }
      if (state.value) {
        state.value = false;
        const ch = stream.peek();
        if (ch === '"' || ch === "'") {
          stream.next();
          state.quote = ch;
          return this.token(stream, state) as Token | null;
        }
        if (stream.match(/^[^\s>]+/)) return 'attributeValue';
      }
      if (stream.eat('=')) {
        state.value = true;
        return 'punctuation';
      }
      if (stream.match(/^[^\s=>/"']+/)) return 'attributeName';
      stream.next();
      return null;
    }
    if (stream.match('<!--')) {
      state.comment = true;
      return this.token(stream, state) as Token | null;
    }
    if (stream.match(/^<\/?(?=[A-Za-z!?])/)) {
      state.tag = true;
      state.name = true;
      return 'punctuation';
    }
    stream.next();
    stream.match(/^[^<]*/);
    return null;
  },
};

export const markupLanguage = streamLanguage(markupTokens, 'markup');
