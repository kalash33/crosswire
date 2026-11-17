// Live shortcut hints. The keymap is plain TS; this module turns its change
// notifications into a reactive version, so any template or `$derived` that
// calls these functions re-renders when a chord is remapped or reset (from
// Settings, while the hint is on screen).
import { formatChord, keymap, type Action } from './keymap';

let version = $state(0);
// The keymap is an app-lifetime singleton, so this subscription never ends.
keymap.subscribe(() => {
  version++;
});

/**
 * The keymap change counter. A `$derived` or template that calls this re-runs on
 * every remap, reset or load (for code that reads the keymap directly).
 */
export function keymapVersion(): number {
  return version;
}

/** Display form of an action's current chord, e.g. "Ctrl+Shift+C"; undefined when unbound. */
export function chordLabel(action: Action): string | undefined {
  void version;
  const chord = keymap.chordFor(action);
  return chord ? formatChord(chord) : undefined;
}

/** Hint for the viewer's find button, e.g. "Ctrl+F in the viewer"; undefined when unbound. */
export function findHint(): string | undefined {
  const label = chordLabel('find');
  return label ? `${label} in the viewer` : undefined;
}
