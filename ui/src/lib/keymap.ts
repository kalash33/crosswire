// Central, remappable keymap. EVERY keyboard shortcut in the app goes through
// this one dispatch table (never per-view keydown handlers): the mouse-free
// core-loop gate depends on that. Views `register` handlers for named actions;
// Shell mounts the single `dispatch` on keydown.
//
// Chords are normalized strings: modifiers in the fixed order ctrl, alt,
// shift, then the key, lowercased, joined by "+" (e.g. "ctrl+shift+]").
// Ctrl is the modifier (not Cmd) so macOS browser Cmd-shortcuts stay out of
// the conflict surface; Meta-modified events never match.

export const ACTIONS = [
  'send-to-repeater',
  'send',
  'forward',
  'drop',
  'toggle-intercept',
  'decode-selection',
  'tab-next',
  'tab-prev',
  'copy-as-curl',
  'copy-raw',
  'show-shortcuts',
] as const;

export type Action = (typeof ACTIONS)[number];
export type Handler = (event: KeyboardEvent) => void;

// Burp Suite defaults where Burp binds the action:
//   send-to-repeater Ctrl+R, forward Ctrl+F, drop Ctrl+D, send Ctrl+Space
//   (Repeater "Send"), toggle-intercept Ctrl+T (Proxy intercept on/off),
//   tab-next/prev Ctrl+Shift+] / [ (Burp's next/previous tab).
// decode-selection has no Burp default; Ctrl+Shift+D (Burp's "send to
// Decoder" neighbourhood) is our consistent Ctrl-chord choice.
export const DEFAULT_BINDINGS: Readonly<Record<Action, string>> = {
  'send-to-repeater': 'ctrl+r',
  send: 'ctrl+space',
  forward: 'ctrl+f',
  drop: 'ctrl+d',
  'toggle-intercept': 'ctrl+t',
  'decode-selection': 'ctrl+shift+d',
  'tab-next': 'ctrl+shift+]',
  'tab-prev': 'ctrl+shift+[',
  // Burp's "Copy as curl command" is Ctrl+Shift+C; both copy UNREDACTED (for
  // replay). The redacted (share-safe) variants are menu-only.
  'copy-as-curl': 'ctrl+shift+c',
  'copy-raw': 'ctrl+shift+s',
  // Plain `?` opens the shortcut cheat sheet (the GitHub/Gmail convention). A
  // chord without Ctrl/Alt never fires while typing in a field (see dispatch).
  'show-shortcuts': '?',
};

/** Human labels for the cheat sheet and Settings (one per action). */
export const ACTION_LABELS: Readonly<Record<Action, string>> = {
  'send-to-repeater': 'Send to Repeater',
  send: 'Send the request (Repeater)',
  forward: 'Forward the held request (Intercept)',
  drop: 'Drop the held request (Intercept)',
  'toggle-intercept': 'Toggle intercept',
  'decode-selection': 'Decode the selection',
  'tab-next': 'Next Repeater tab',
  'tab-prev': 'Previous Repeater tab',
  'copy-as-curl': 'Copy as curl',
  'copy-raw': 'Copy the raw request',
  'show-shortcuts': 'Show keyboard shortcuts',
};

/** One row of the shortcut table. */
export interface Binding {
  action: Action;
  chord: string;
  label: string;
}

/** Display form of a normalized chord: `ctrl+shift+d` -> `Ctrl+Shift+D`. */
export function formatChord(chord: string): string {
  return chord
    .split(/\+(?=.)/)
    .map((p) => (p.length === 1 ? p.toUpperCase() : p.charAt(0).toUpperCase() + p.slice(1)))
    .join('+');
}

/** True when keystrokes on `target` are text entry (fields, CodeMirror). */
function isEditable(target: EventTarget | null): boolean {
  if (!(target instanceof Element)) return false;
  return (
    target.closest('input, textarea, select, [contenteditable]:not([contenteditable="false"])') !==
    null
  );
}

const CODE_KEYS: Record<string, string> = {
  BracketRight: ']',
  BracketLeft: '[',
  Space: 'space',
};

/** Normalize a keyboard event; null when Meta is held or only a modifier. */
export function chordFromEvent(e: KeyboardEvent): string | null {
  if (e.metaKey) return null;
  const fromCode = CODE_KEYS[e.code];
  let k = fromCode ?? (e.key === ' ' ? 'space' : e.key.toLowerCase());
  if (['control', 'shift', 'alt', 'meta'].includes(k)) return null;
  // Shift changes e.key ("}" for "]"); letters resolve from the physical code.
  const letter = /^Key[A-Z]$/.test(e.code);
  if (letter) k = e.code.slice(3).toLowerCase();
  // A shifted symbol ("?", "!") already encodes Shift in its character, and
  // layouts differ on whether it needs Shift at all: drop it from the chord.
  const shiftInKey = !fromCode && !letter && k.length === 1 && !/[a-z0-9]/.test(k);
  const parts: string[] = [];
  if (e.ctrlKey) parts.push('ctrl');
  if (e.altKey) parts.push('alt');
  if (e.shiftKey && !shiftInKey) parts.push('shift');
  parts.push(k);
  return parts.join('+');
}

export function createKeymap(initial: Readonly<Record<Action, string>> = DEFAULT_BINDINGS) {
  const bindings = new Map<Action, string>(Object.entries(initial) as [Action, string][]);
  const handlers = new Map<Action, Handler>();

  function actionFor(chord: string): Action | undefined {
    for (const [action, c] of bindings) if (c === chord) return action;
    return undefined;
  }

  return {
    chordFor(action: Action): string | undefined {
      return bindings.get(action);
    },
    /** Every action with its current chord and label, in ACTIONS order. */
    bindings(): Binding[] {
      const out: Binding[] = [];
      for (const action of ACTIONS) {
        const chord = bindings.get(action);
        if (chord) out.push({ action, chord, label: ACTION_LABELS[action] });
      }
      return out;
    },
    /** Register the handler for an action (one per action; last wins). */
    register(action: Action, handler: Handler): () => void {
      handlers.set(action, handler);
      return () => {
        if (handlers.get(action) === handler) handlers.delete(action);
      };
    },
    /** Rebind an action; a chord already held by another action is taken over. */
    remap(action: Action, chord: string): void {
      const norm = chord.toLowerCase();
      const holder = actionFor(norm);
      if (holder && holder !== action) bindings.delete(holder);
      bindings.set(action, norm);
    },
    /**
     * The single keydown entry point. Only a chord bound to an action WITH a
     * registered handler is preventDefault'd and dispatched; anything else is
     * left to the browser untouched.
     */
    dispatch(e: KeyboardEvent): void {
      const chord = chordFromEvent(e);
      if (chord === null) return;
      // A plain-key chord (no Ctrl/Alt) is text while the user is typing.
      if (!/^(ctrl|alt)\+/.test(chord) && isEditable(e.target)) return;
      const action = actionFor(chord);
      if (!action) return;
      const handler = handlers.get(action);
      if (!handler) return;
      e.preventDefault();
      handler(e);
    },
  };
}

export const keymap = createKeymap();
