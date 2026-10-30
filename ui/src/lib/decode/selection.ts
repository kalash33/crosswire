import { EditorView } from '@codemirror/view';
import { encodePastedText, latin1ToBytes } from '../editor/encoding';

/**
 * The bytes of the current selection. Inside a CodeMirror editor the DOM text
 * contains placeholder glyphs for non-printables and omits off-viewport lines,
 * so read the editor state instead (byte-exact under the 1:1 Latin-1 mapping).
 * Elsewhere fall back to the DOM selection text: exact Latin-1 when it fits,
 * else UTF-8 with `nonLatin1` set so the UI can say it re-encoded.
 */
export function selectionBytes(): { bytes: Uint8Array; nonLatin1: boolean } {
  const sel = window.getSelection();
  const anchor = sel?.anchorNode ?? null;
  const el = anchor instanceof Element ? anchor : (anchor?.parentElement ?? null);
  const editorEl = el?.closest('.cm-editor');
  if (editorEl instanceof HTMLElement) {
    const view = EditorView.findFromDOM(editorEl);
    if (view) {
      const { from, to } = view.state.selection.main;
      return { bytes: latin1ToBytes(view.state.sliceDoc(from, to)), nonLatin1: false };
    }
  }
  const text = sel?.toString() ?? '';
  const pasted = encodePastedText(text);
  return pasted.nonLatin1 ? pasted : { bytes: latin1ToBytes(text), nonLatin1: false };
}
