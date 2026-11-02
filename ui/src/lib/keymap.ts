// Central, remappable keymap. EVERY keyboard shortcut in the app goes through
// this one dispatch table (never per-view keydown handlers): the mouse-free
// core-loop gate depends on that. Views `register` handlers for named actions;
// Shell mounts the single `dispatch` on keydown.
//
// Chords are normalized strings: modifiers in the fixed order ctrl, alt,
// shift, then the key, lowercased, joined by "+" (e.g. "ctrl+shift+]").
// Ctrl is the modifier (not Cmd) so macOS browser Cmd-shortcuts stay out of
// the conflict surface; Meta-modified events never match.
//
// Scopes (V1): every action belongs to one scope. A chord is unique only among
// scopes that can be active at the same time ("co-active"), so the viewer's
// Find and Intercept's Forward can both be Ctrl+F. At dispatch the innermost
// focused scope wins: the chain is the `data-keyscope` ancestors of the event
// target, then the route's scope, then `global`.

/** Keymap scopes in display order (Settings and the `?` sheet group by these). */
export const SCOPES = ['global', 'history', 'repeater', 'intercept', 'viewer'] as const;
export type Scope = (typeof SCOPES)[number];

export const SCOPE_LABELS: Readonly<Record<Scope, string>> = {
  global: 'Everywhere',
  history: 'History',
  repeater: 'Repeater',
  intercept: 'Intercept',
  viewer: 'Message viewer',
};

/**
 * Where a scope can be active. The route scopes are mutually exclusive (one
 * view is mounted); the viewer lives inside History (detail) and Repeater
 * (response). If Intercept ever embeds a viewer, `viewer` gains `intercept`,
 * which makes Find and Forward clash on Ctrl+F: rebind one in the same change
 * (the "defaults have no co-active clash" test fails until then).
 */
export const PARENTS: Readonly<Record<Scope, readonly Scope[]>> = {
  global: [],
  history: ['global'],
  repeater: ['global'],
  intercept: ['global'],
  viewer: ['history', 'repeater', 'global'],
};

/** True when `a` and `b` can be active together, so a chord must not be shared. */
export function coActive(a: Scope, b: Scope): boolean {
  return a === b || PARENTS[b].includes(a) || PARENTS[a].includes(b);
}

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
  'focus-search',
  'find',
] as const;

export type Action = (typeof ACTIONS)[number];

/**
 * A key handler. Returning `false` declines the key: it is not
 * preventDefault'd and no outer scope sees it, so the browser default runs.
 */
export type Handler = (event: KeyboardEvent) => void | boolean;

export const ACTION_SCOPES: Readonly<Record<Action, Scope>> = {
  // Global: reserved app-wide; whichever view is mounted supplies the handler.
  'send-to-repeater': 'global',
  'toggle-intercept': 'global',
  'decode-selection': 'global',
  'copy-as-curl': 'global',
  'copy-raw': 'global',
  'show-shortcuts': 'global',
  'focus-search': 'history',
  send: 'repeater',
  'tab-next': 'repeater',
  'tab-prev': 'repeater',
  forward: 'intercept',
  drop: 'intercept',
  find: 'viewer',
};

/**
 * Actions added after the U2 storage format. A stored U2 override that lands
 * on one of their defaults was legal when it was saved, so load keeps both and
 * innermost-scope-wins resolves the key (see `shadows`).
 */
const POST_U2_ACTIONS: ReadonlySet<Action> = new Set<Action>(['focus-search', 'find']);

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
  // `/` focuses the History search (the GitHub/Gmail convention).
  'focus-search': '/',
  // The viewer's find shares Ctrl+F with Forward: the scopes never overlap.
  find: 'ctrl+f',
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
  'focus-search': 'Focus the History search',
  find: 'Find in the message',
};

/** One row of the shortcut table; `chord` is null when the action is unbound. */
export interface Binding {
  action: Action;
  chord: string | null;
  label: string;
  scope: Scope;
}

/** Display form of a normalized chord: `ctrl+shift+d` -> `Ctrl+Shift+D`. */
export function formatChord(chord: string): string {
  return chord
    .split(/\+(?=.)/)
    .map((p) => (p.length === 1 ? p.toUpperCase() : p.charAt(0).toUpperCase() + p.slice(1)))
    .join('+');
}

/** The route scope for a router path; null where no view scope applies. */
export function routeScopeFor(path: string): Scope | null {
  switch (path) {
    case '/':
      return 'history';
    case '/repeater':
      return 'repeater';
    case '/intercept':
      return 'intercept';
    default:
      return null;
  }
}

function isScope(v: string | null): v is Scope {
  return v !== null && (SCOPES as readonly string[]).includes(v);
}

/**
 * The scopes a key on `target` passes through, innermost first: the
 * `data-keyscope` values of the target and its ancestors, then `routeScope`
 * (covers focus outside the view: body, top bar, rail), then `global`.
 */
export function scopeChain(target: EventTarget | null, routeScope: Scope | null): Scope[] {
  const chain: Scope[] = [];
  const push = (s: Scope) => {
    if (!chain.includes(s)) chain.push(s);
  };
  let el: Element | null =
    target instanceof Element ? target : target instanceof Node ? target.parentElement : null;
  while (el) {
    const hit: Element | null = el.closest('[data-keyscope]');
    if (!hit) break;
    const s = hit.getAttribute('data-keyscope');
    if (isScope(s)) push(s);
    el = hit.parentElement;
  }
  if (routeScope) push(routeScope);
  push('global');
  return chain;
}

/** True when keystrokes on `target` are text entry (fields, CodeMirror). */
function isEditable(target: EventTarget | null): boolean {
  if (!(target instanceof Element)) return false;
  return (
    target.closest('input, textarea, select, [contenteditable]:not([contenteditable="false"])') !==
    null
  );
}

/** True inside an open menu or dialog, where plain keys belong to the widget. */
function inMenuOrDialog(target: EventTarget | null): boolean {
  return target instanceof Element && target.closest('[role="menu"], [role="dialog"]') !== null;
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
  // An action absent from `bindings` is unbound (its chord was taken over).
  const bindings = new Map<Action, string>(Object.entries(initial) as [Action, string][]);
  const handlers = new Map<Action, Handler>();
  const bound = new Map<Action, { el: Element; handler: Handler }[]>();
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

  const overlap = (a: Action, b: Action): boolean => coActive(ACTION_SCOPES[a], ACTION_SCOPES[b]);

  function loadOverrides(): void {
    if (storageKey === null) return;
    const stored = readOverrides(storageKey);
    // Resolve against the FINAL override set (not storage order): drop an
    // override whose chord a co-active action ends up holding, until stable.
    // A post-U2 action still on its default does not count: that clash was
    // legal when the override was saved, so both are kept as a shadow.
    for (;;) {
      const finalChord = (a: Action): string => stored[a] ?? defaults[a];
      const clash = ACTIONS.find(
        (a) =>
          stored[a] !== undefined &&
          ACTIONS.some(
            (o) =>
              o !== a &&
              overlap(o, a) &&
              finalChord(o) === stored[a] &&
              (stored[o] === undefined ? !POST_U2_ACTIONS.has(o) : o < a),
          ),
      );
      if (!clash) break;
      delete stored[clash];
    }
    for (const [action, chord] of Object.entries(stored) as [Action, string][]) {
      bindings.set(action, chord);
      if (chord !== defaults[action]) overrides.set(action, chord);
    }
  }

  /** Other actions on `chord` whose scope is co-active with `action`'s, in ACTIONS order. */
  function holders(chord: string, action: Action): Action[] {
    return ACTIONS.filter((o) => o !== action && bindings.get(o) === chord && overlap(o, action));
  }

  /** The action of `scope` bound to `chord` (at most one per scope). */
  function actionIn(scope: Scope, chord: string): Action | undefined {
    return ACTIONS.find((a) => ACTION_SCOPES[a] === scope && bindings.get(a) === chord);
  }

  /** The innermost element-bound handler containing `target`, else the unbound one. */
  function handlerFor(action: Action, target: EventTarget | null): Handler | undefined {
    if (target instanceof Node) {
      let best: { el: Element; handler: Handler } | undefined;
      for (const b of bound.get(action) ?? []) {
        if (b.el.contains(target) && (!best || best.el.contains(b.el))) best = b;
      }
      if (best) return best.handler;
    }
    return handlers.get(action);
  }

  /**
   * Scopes to try for a key. With a route scope (or an explicit null) this is
   * `scopeChain`. Without one (callers that do not know the route yet) every
   * scope follows the DOM chain, as U2's single flat table did: a view-scoped
   * action fires wherever its view registered a handler.
   */
  function chainFor(target: EventTarget | null, routeScope: Scope | null | undefined): Scope[] {
    if (routeScope !== undefined) return scopeChain(target, routeScope);
    const chain = scopeChain(target, null);
    for (const s of SCOPES) if (!chain.includes(s)) chain.push(s);
    return chain;
  }

  loadOverrides();

  return {
    chordFor(action: Action): string | undefined {
      return bindings.get(action);
    },
    /** Every action in ACTIONS order with its chord (null when unbound), label and scope. */
    bindings(): Binding[] {
      return ACTIONS.map((action) => ({
        action,
        chord: bindings.get(action) ?? null,
        label: ACTION_LABELS[action],
        scope: ACTION_SCOPES[action],
      }));
    },
    /**
     * Actions in outer scopes on the same chord that `action` hides while its
     * scope is focused (e.g. Find over a global action moved onto Ctrl+F).
     * Only loaded U2 data can produce one; interactive remaps never do.
     */
    shadows(action: Action): Action[] {
      const chord = bindings.get(action);
      if (chord === undefined) return [];
      const outer = PARENTS[ACTION_SCOPES[action]];
      return ACTIONS.filter((o) => o !== action && bindings.get(o) === chord && outer.includes(ACTION_SCOPES[o]));
    },
    /**
     * Register the handler for an action. Without `el`: one per action, last
     * wins. With `el`: bound to that element; any number per action, and a key
     * uses the innermost one whose element contains the event target.
     */
    register(action: Action, handler: Handler, opts: { el?: Element } = {}): () => void {
      const el = opts.el;
      if (el) {
        const entry = { el, handler };
        const list = bound.get(action) ?? [];
        list.push(entry);
        bound.set(action, list);
        return () => {
          const cur = bound.get(action);
          if (!cur) return;
          const i = cur.indexOf(entry);
          if (i >= 0) cur.splice(i, 1);
          if (cur.length === 0) bound.delete(action);
        };
      }
      handlers.set(action, handler);
      return () => {
        if (handlers.get(action) === handler) handlers.delete(action);
      };
    },
    /**
     * Rebind an action and persist it. A chord already held by a co-active
     * action is taken over (that action goes unbound and is not persisted);
     * holders in disjoint scopes keep it. Callers that must not steal check
     * `validateChord` first.
     */
    remap(action: Action, chord: string): void {
      const norm = normalizeChord(chord) ?? chord.toLowerCase();
      for (const holder of holders(norm, action)) {
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
     * If a restored default chord is held by a co-active action, that holder's
     * own override is dropped too (cascading) so two co-active actions never
     * share a chord.
     */
    reset(action?: Action): void {
      const queue: Action[] = action ? [action] : [...ACTIONS];
      while (queue.length > 0) {
        const a = queue.shift() as Action;
        overrides.delete(a);
        const def = defaults[a];
        for (const holder of holders(def, a)) {
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
    /**
     * Refuse reserved, modifier-less and conflicting chords for `action`. Only
     * a co-active holder conflicts; the message names the first one in ACTIONS
     * order and its scope.
     */
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
      const holder = holders(norm, action)[0];
      if (holder) {
        return {
          ok: false,
          reason: `${formatChord(norm)} is already used by "${ACTION_LABELS[holder]}" (${SCOPE_LABELS[ACTION_SCOPES[holder]]}).`,
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
     * The single keydown entry point. Walks the scope chain innermost first;
     * the first scope whose action on this chord has a handler takes the key:
     * it is preventDefault'd, unless the handler declines (returns false), in
     * which case the browser keeps it. Either way no outer scope sees it.
     * Anything else is left to the browser untouched.
     */
    dispatch(e: KeyboardEvent, routeScope?: Scope | null): void {
      const chord = chordFromEvent(e);
      if (chord === null) return;
      // A plain-key chord (no Ctrl/Alt) is text while the user is typing, and
      // belongs to the widget inside an open menu or dialog.
      if (!/^(ctrl|alt)\+/.test(chord) && (isEditable(e.target) || inMenuOrDialog(e.target))) return;
      for (const scope of chainFor(e.target, routeScope)) {
        const action = actionIn(scope, chord);
        if (!action) continue;
        const handler = handlerFor(action, e.target);
        if (!handler) continue;
        if (handler(e) !== false) e.preventDefault();
        return;
      }
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
