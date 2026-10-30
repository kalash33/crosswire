import { EditorView } from '@codemirror/view';
import { latin1ToBytes } from '../editor/encoding';

/**
 * The bytes of the current selection. Inside a CodeMirror editor the DOM text
 * contains placeholder glyphs for non-printables and omits off-viewport lines,
 * so read the editor state instead (byte-exact under the 1:1 Latin-1 mapping).
 * Elsewhere fall back to the DOM selection text.
 */
export function selectionBytes(): Uint8Array {
  const sel = window.getSelection();
  const anchor = sel?.anchorNode ?? null;
  const el = anchor instanceof Element ? anchor : (anchor?.parentElement ?? null);
  const editorEl = el?.closest('.cm-editor');
  if (editorEl instanceof HTMLElement) {
    const view = EditorView.findFromDOM(editorEl);
    if (view) {
      const { from, to } = view.state.selection.main;
      return latin1ToBytes(view.state.sliceDoc(from, to));
    }
  }
  return latin1ToBytes(sel?.toString() ?? '');
}
