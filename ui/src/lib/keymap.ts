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
//   (Repeater "Send"), toggle-intercept Ctrl+I (Burp's Ctrl+T is the browser's new-tab chord and
//   is not interceptable in a web page),
//   tab-next/prev Ctrl+Shift+] / [ (Burp's next/previous tab).
// decode-selection has no Burp default; Ctrl+Shift+D (Burp's "send to
// Decoder" neighbourhood) is our consistent Ctrl-chord choice.
export const DEFAULT_BINDINGS: Readonly<Record<Action, string>> = {
  'send-to-repeater': 'ctrl+r',
  send: 'ctrl+space',
  forward: 'ctrl+f',
  drop: 'ctrl+d',
  'toggle-intercept': 'ctrl+i',
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


/** localStorage key holding the persisted `{action: chord}` overrides. */
export const KEYMAP_KEY = 'crosswire.keymap';

const MOD_ORDER = ['ctrl', 'alt', 'shift'] as const;

/**
 * Canonical chord string: modifiers ordered ctrl, alt, shift, the key lower-
 * cased, `Space`/" " as `space`, brackets as-is. Returns null for an empty or
 * modifier-only chord, an unknown modifier, or a Meta chord (unsupported).
 */
export function normalizeChord(chord: string): string | null {
  const raw = chord === ' ' ? 'space' : chord;
  if (raw.trim() === '') return null;
  // The key is the last segment; a literal "+" key is "...++" or just "+".
  const m = /^(.*?)\+?(\+|[^+]+)$/.exec(raw);
  if (!m) return null;
  const mods = m[1] === '' ? [] : m[1].split('+').map((x) => x.trim().toLowerCase());
  let key = m[2].trim().toLowerCase();
  if (key === '') key = m[2] === ' ' ? 'space' : '';
  if (key === '') return null;
  if (['control', 'ctrl', 'shift', 'alt', 'meta', 'cmd', 'command'].includes(key)) return null;
  const set = new Set<string>();
  for (const mod of mods) {
    if (!(MOD_ORDER as readonly string[]).includes(mod)) return null;
    set.add(mod);
  }
  const ordered = MOD_ORDER.filter((x) => set.has(x));
  return [...ordered, key].join('+');
}

/**
 * Chords a web page cannot reliably intercept (browser tab/window commands).
 * Meta equivalents are not listed: Meta-modified events never match (see
 * chordFromEvent), so they only become relevant if Meta is ever supported.
 */
const RESERVED = new Set([
  'ctrl+t',
  'ctrl+n',
  'ctrl+w',
  'ctrl+shift+t',
  'ctrl+shift+n',
  'ctrl+shift+w',
]);

export type ChordCheck = { ok: true } | { ok: false; reason: string };

export interface KeymapOptions {
  /** localStorage key to load/save overrides under; null/omitted: memory only. */
  storageKey?: string | null;
}

function isAction(v: string): v is Action {
  return (ACTIONS as readonly string[]).includes(v);
}

/** The overrides actually applied from storage; unknown/invalid entries dropped. */
function readOverrides(storageKey: string): Partial<Record<Action, string>> {
  const out: Partial<Record<Action, string>> = {};
  let parsed: unknown;
  try {
    const raw = localStorage.getItem(storageKey);
    if (raw === null) return out;
    parsed = JSON.parse(raw);
  } catch {
    // Storage disabled or corrupt JSON: fall back to the defaults.
    return out;
  }
  if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) return out;
  for (const [action, value] of Object.entries(parsed as Record<string, unknown>)) {
    if (!isAction(action) || typeof value !== 'string') continue;
    const norm = normalizeChord(value);
    if (norm === null || RESERVED.has(norm) || modifierless(norm)) continue;
    out[action] = norm;
  }
  return out;
}

/** A bare letter/digit (optionally with only Shift) would fire while typing. */
function modifierless(chord: string): boolean {
  const parts = chord.split(/\+(?=.)/);
  const key = parts[parts.length - 1];
  const hasCtrlAlt = parts.includes('ctrl') || parts.includes('alt');
  // Only a single printable symbol (the "?" case) may stand without Ctrl/Alt;
  // letters, digits and named keys (tab, enter, space, f1...) would hijack
  // typing or focus navigation app-wide.
  return !hasCtrlAlt && !(key.length === 1 && !/[a-z0-9]/.test(key));
}

export function createKeymap(
  initial: Readonly<Record<Action, string>> = DEFAULT_BINDINGS,
  options: KeymapOptions = {},
) {
  const storageKey = options.storageKey ?? null;
  const defaults = initial;
  const bindings = new Map<Action, string>(Object.entries(initial) as [Action, string][]);
  const handlers = new Map<Action, Handler>();
  const overrides = new Map<Action, string>();
  const listeners = new Set<() => void>();

  function notify(): void {
    for (const l of listeners) l();
  }

  function saveOverrides(): void {
    if (storageKey === null) return;
    try {
      if (overrides.size === 0) localStorage.removeItem(storageKey);
      else localStorage.setItem(storageKey, JSON.stringify(Object.fromEntries(overrides)));
    } catch {
      // Not persisted; the binding still applies for this page load.
    }
  }

  function loadOverrides(): void {
    if (storageKey === null) return;
    const stored = readOverrides(storageKey);
    // Resolve against the FINAL override set (not storage order): drop an
    // override whose chord another action ends up holding, until stable.
    for (;;) {
      const finalChord = (a: Action): string => stored[a] ?? defaults[a];
      const clash = ACTIONS.find(
        (a) =>
          stored[a] !== undefined &&
          ACTIONS.some((o) => o !== a && finalChord(o) === stored[a] && (stored[o] === undefined || o < a)),
      );
      if (!clash) break;
      delete stored[clash];
    }
    for (const [action, chord] of Object.entries(stored) as [Action, string][]) {
      bindings.set(action, chord);
      if (chord !== defaults[action]) overrides.set(action, chord);
    }
  }

  function actionFor(chord: string): Action | undefined {
    for (const [action, c] of bindings) if (c === chord) return action;
    return undefined;
  }

  loadOverrides();

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
    /**
     * Rebind an action and persist it. A chord already held by another action
     * is taken over (that action goes unbound and is not persisted); callers
     * that must not steal check `validateChord` first.
     */
    remap(action: Action, chord: string): void {
      const norm = normalizeChord(chord) ?? chord.toLowerCase();
      const holder = actionFor(norm);
      if (holder && holder !== action) {
        bindings.delete(holder);
        overrides.delete(holder);
      }
      bindings.set(action, norm);
      if (norm === defaults[action]) overrides.delete(action);
      else overrides.set(action, norm);
      saveOverrides();
      notify();
    },
    /**
     * Drop one override (or all, with no argument) and restore the defaults.
     * If a restored default chord is held by another action, that holder's own
     * override is dropped too (cascading) so two actions never share a chord.
     */
    reset(action?: Action): void {
      const queue: Action[] = action ? [action] : [...ACTIONS];
      while (queue.length > 0) {
        const a = queue.shift() as Action;
        overrides.delete(a);
        const def = defaults[a];
        const holder = actionFor(def);
        if (holder && holder !== a) {
          // Another action sits on this default: send it back to its own.
          if (overrides.has(holder)) queue.push(holder);
          else bindings.delete(holder);
        }
        bindings.set(a, def);
      }
      saveOverrides();
      notify();
    },
    /** Actions whose chord differs from the default (for the "custom" badge). */
    isOverridden(action: Action): boolean {
      return overrides.has(action);
    },
    /** Reload overrides from storage over the defaults (startup, tests). */
    loadOverrides(): void {
      loadOverrides();
      notify();
    },
    saveOverrides,
    /** Refuse reserved, modifier-less and conflicting chords for `action`. */
    validateChord(chord: string, action: Action): ChordCheck {
      const norm = normalizeChord(chord);
      if (norm === null) return { ok: false, reason: 'That is not a valid shortcut.' };
      if (RESERVED.has(norm)) {
        return {
          ok: false,
          reason: `${formatChord(norm)} is reserved by the browser and cannot be used.`,
        };
      }
      if (modifierless(norm)) {
        return { ok: false, reason: 'Add Ctrl or Alt: a bare key would fire while you type or break navigation.' };
      }
      const holder = actionFor(norm);
      if (holder && holder !== action) {
        return {
          ok: false,
          reason: `${formatChord(norm)} is already used by "${ACTION_LABELS[holder]}".`,
        };
      }
      return { ok: true };
    },
    /** Subscribe to binding changes (remap/reset/load); returns unsubscribe. */
    subscribe(fn: () => void): () => void {
      listeners.add(fn);
      return () => listeners.delete(fn);
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

export const keymap = createKeymap(DEFAULT_BINDINGS, { storageKey: KEYMAP_KEY });

/** What one keydown means while the user is recording a new shortcut. */
export type Recording =
  | { kind: 'ignore' }
  | { kind: 'cancel' }
  | { kind: 'refused'; chord: string; reason: string }
  | { kind: 'ok'; chord: string };

/**
 * Interpret the next keydown of a recording session: lone modifiers are
 * ignored, a bare Escape cancels, anything else is validated for `action`.
 */
export function recordChord(
  e: KeyboardEvent,
  action: Action,
  km: Pick<ReturnType<typeof createKeymap>, 'validateChord'> = keymap,
): Recording {
  if (['Control', 'Shift', 'Alt', 'Meta'].includes(e.key)) return { kind: 'ignore' };
  if (e.key === 'Escape' && !e.ctrlKey && !e.altKey && !e.shiftKey && !e.metaKey) {
    return { kind: 'cancel' };
  }
  const chord = chordFromEvent(e);
  if (chord === null) {
    return { kind: 'refused', chord: '', reason: 'Meta (Cmd/Windows) shortcuts are not supported.' };
  }
  const check = km.validateChord(chord, action);
  return check.ok ? { kind: 'ok', chord } : { kind: 'refused', chord, reason: check.reason };
}
