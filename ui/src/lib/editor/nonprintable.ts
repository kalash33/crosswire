// Non-printable byte placeholders, rendered as VIEW-ONLY CodeMirror decorations.
//
// §7.2: "non-printables appear as visible placeholders". The underlying
// document always holds the real bytes (via the Latin-1 mapping); this plugin
// only changes what is *drawn*. Replace decorations swap the glyph in the view
// while leaving the document text untouched, so reading the bytes back
// (doc.toString() -> latin1ToBytes) returns the originals, never a placeholder.
//
// Decorations are computed over the visible ranges only (the idiomatic CM
// pattern) so large buffers stay cheap to render.

import {
  Decoration,
  type DecorationSet,
  EditorView,
  ViewPlugin,
  type ViewUpdate,
  WidgetType,
} from '@codemirror/view';
import { RangeSetBuilder } from '@codemirror/state';

// A code point is "printable" iff it is ASCII graphic (0x20-0x7E) or Latin-1
// graphic (0xA0-0xFF). Everything else is a control/non-printable byte.
// 0x0A (LF) is deliberately NOT treated as non-printable: it is the structural
// line separator and is rendered as a real line break. 0x0D (CR), 0x09 (TAB),
// the other C0 controls, 0x7F (DEL) and the C1 controls (0x80-0x9F) all get a
// placeholder.
function isNonPrintable(code: number): boolean {
  if (code === 0x0a) return false;
  if (code < 0x20) return true;
  if (code === 0x7f) return true;
  if (code >= 0x80 && code <= 0x9f) return true;
  return false;
}

// A short, unambiguous glyph for a non-printable byte. C0 controls and DEL use
// the Unicode Control Pictures block (␀..␟, ␡); C1 controls have no such glyph,
// so they show as \xNN.
function label(code: number): string {
  if (code <= 0x1f) return String.fromCharCode(0x2400 + code);
  if (code === 0x7f) return '␡';
  return '\\x' + code.toString(16).toUpperCase().padStart(2, '0');
}

class PlaceholderWidget extends WidgetType {
  constructor(readonly code: number) {
    super();
  }

  eq(other: PlaceholderWidget): boolean {
    return other.code === this.code;
  }

  toDOM(): HTMLElement {
    // textContent only — never innerHTML — so nothing from the byte stream is
    // ever interpreted as markup (§11.1).
    const span = document.createElement('span');
    // CR also carries `cm-ctl`: the viewer's "control characters" toggle
    // hides `.cm-ctl` glyphs with CSS (the byte stays in the document).
    span.className = this.code === 0x0d ? 'cm-nonprintable cm-ctl' : 'cm-nonprintable';
    span.textContent = label(this.code);
    span.setAttribute('data-byte', String(this.code));
    span.title = '0x' + this.code.toString(16).toUpperCase().padStart(2, '0');
    return span;
  }

  ignoreEvent(): boolean {
    return false;
  }
}

function buildDecorations(view: EditorView): DecorationSet {
  const builder = new RangeSetBuilder<Decoration>();
  for (const { from, to } of view.visibleRanges) {
    const text = view.state.doc.sliceString(from, to);
    for (let i = 0; i < text.length; i++) {
      const code = text.charCodeAt(i);
      if (isNonPrintable(code)) {
        const pos = from + i;
        builder.add(pos, pos + 1, Decoration.replace({ widget: new PlaceholderWidget(code) }));
      }
    }
  }
  return builder.finish();
}

export const nonPrintablePlaceholders = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;

    constructor(view: EditorView) {
      this.decorations = buildDecorations(view);
    }

    update(u: ViewUpdate): void {
      if (u.docChanged || u.viewportChanged) {
        this.decorations = buildDecorations(u.view);
      }
    }
  },
  { decorations: (v) => v.decorations },
);

// LF markers for the viewer's "show control characters" toggle (U2 D1). An LF
// is the line break itself, so it is never replaced; this adds a `␊` glyph
// WIDGET at the end of each line that ends in LF. Hosts that do not opt in
// (editable editors) never load it, so their text is unchanged.
class LineEndWidget extends WidgetType {
  eq(): boolean {
    return true;
  }

  toDOM(): HTMLElement {
    const span = document.createElement('span');
    span.className = 'cm-nonprintable cm-ctl';
    span.textContent = '␊';
    span.setAttribute('data-byte', '10');
    span.title = '0x0A';
    return span;
  }
}

const LF_MARK = Decoration.widget({ widget: new LineEndWidget(), side: -1 });

function buildLineEnds(view: EditorView): DecorationSet {
  const builder = new RangeSetBuilder<Decoration>();
  for (const { from, to } of view.visibleRanges) {
    const text = view.state.doc.sliceString(from, to);
    for (let i = text.indexOf('\n'); i >= 0; i = text.indexOf('\n', i + 1)) {
      builder.add(from + i, from + i, LF_MARK);
    }
  }
  return builder.finish();
}

export const lineEndMarkers = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;

    constructor(view: EditorView) {
      this.decorations = buildLineEnds(view);
    }

    update(u: ViewUpdate): void {
      if (u.docChanged || u.viewportChanged) {
        this.decorations = buildLineEnds(u.view);
      }
    }
  },
  { decorations: (v) => v.decorations },
);
