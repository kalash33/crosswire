// Syntax colouring for the message viewer (U2 D1), CSP-clean and text-only.
//
// Each tokenizer (json.ts, http.ts, markup.ts) is a hand-written CodeMirror
// stream parser that returns a name from the fixed TOKENS set below, never
// markup. `streamLanguage` wraps one in a StreamLanguage. Colouring is done by
// `tokenClasses`: a view plugin that walks the parsed syntax tree over the
// visible ranges and adds mark decorations whose ONLY output is a class
// `tok-<token>` from the same fixed set. The colours live in highlight.css as
// theme tokens (`--syn-*` in theme.css).
//
// Why not a HighlightStyle: it needs `Tag` objects from @lezer/highlight,
// which is not a declared dependency (U2 D6 promotes only @codemirror/language
// and @codemirror/streamparser). The class-only plugin also injects no style
// element at all, so it needs no CSP nonce.

import { StreamLanguage, syntaxTree, type StreamParser } from '@codemirror/language';
import { RangeSetBuilder, type Extension } from '@codemirror/state';
import { Decoration, type DecorationSet, EditorView, ViewPlugin, type ViewUpdate } from '@codemirror/view';

export const TOKENS = [
  'string',
  'number',
  'keyword',
  'propertyName',
  'punctuation',
  'comment',
  'tagName',
  'attributeName',
  'attributeValue',
  'headerName',
  'headerValue',
  'statusLine',
] as const;

export type Token = (typeof TOKENS)[number];

// StreamLanguage turns a token name into a syntax node only when the name is
// a standard highlighting tag, and names the node after it. Each of our
// tokens maps to a DISTINCT standard tag, so a node name maps back to exactly
// one token.
const NODE_NAME: Readonly<Record<Token, string>> = {
  string: 'string',
  number: 'number',
  keyword: 'keyword',
  propertyName: 'propertyName',
  punctuation: 'punctuation',
  comment: 'comment',
  tagName: 'tagName',
  attributeName: 'attributeName',
  attributeValue: 'attributeValue',
  headerName: 'labelName',
  headerValue: 'content',
  statusLine: 'heading',
};

const TOKEN_OF_NODE: ReadonlyMap<string, Token> = new Map(
  TOKENS.map((t) => [NODE_NAME[t], t] as [string, Token]),
);

const MARKS: ReadonlyMap<string, Decoration> = new Map(
  TOKENS.map((t) => [NODE_NAME[t], Decoration.mark({ class: `tok-${t}` })] as [string, Decoration]),
);

/** Wrap a tokenizer returning TOKENS names into a StreamLanguage. */
export function streamLanguage<S>(parser: StreamParser<S>, name: string): StreamLanguage<S> {
  return StreamLanguage.define<S>({
    ...parser,
    name,
    token(stream, state) {
      const t = parser.token(stream, state) as Token | null;
      return t ? NODE_NAME[t] : null;
    },
  });
}

/** The token a syntax node stands for (exposed for tests). */
export function tokenOfNode(name: string): Token | undefined {
  return TOKEN_OF_NODE.get(name);
}

function build(view: EditorView): DecorationSet {
  const b = new RangeSetBuilder<Decoration>();
  const tree = syntaxTree(view.state);
  for (const { from, to } of view.visibleRanges) {
    tree.iterate({
      from,
      to,
      enter(node) {
        const mark = MARKS.get(node.type.name);
        if (mark && node.to > node.from) b.add(node.from, node.to, mark);
      },
    });
  }
  return b.finish();
}

/** Class-only colouring from the syntax tree (see the header comment). */
export const tokenClasses: Extension = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;
    constructor(view: EditorView) {
      this.decorations = build(view);
    }
    update(u: ViewUpdate): void {
      if (u.docChanged || u.viewportChanged || syntaxTree(u.state) !== syntaxTree(u.startState)) {
        this.decorations = build(u.view);
      }
    }
  },
  { decorations: (v) => v.decorations },
);
