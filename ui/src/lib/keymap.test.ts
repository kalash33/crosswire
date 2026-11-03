import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  ACTION_LABELS,
  ACTION_SCOPES,
  ACTIONS,
  coActive,
  createKeymap,
  DEFAULT_BINDINGS,
  chordFromEvent,
  formatChord,
  normalizeChord,
  PARENTS,
  recordChord,
  routeScopeFor,
  SCOPE_LABELS,
  SCOPES,
  scopeChain,
  type Scope,
} from './keymap';

function key(init: KeyboardEventInit): KeyboardEvent {
  return new KeyboardEvent('keydown', { cancelable: true, bubbles: true, ...init });
}

describe('keymap defaults (Burp-consistent)', () => {
  it('binds exactly the named actions to the chosen chords', () => {
    expect([...ACTIONS].sort()).toEqual(
      [
        'copy-as-curl',
        'copy-raw',
        'decode-selection',
        'drop',
        'forward',
        'send',
        'send-to-repeater',
        'tab-next',
        'tab-prev',
        'toggle-intercept',
        'show-shortcuts',
        'focus-search',
        'find',
      ].sort(),
    );
    expect(DEFAULT_BINDINGS).toEqual({
      'send-to-repeater': 'ctrl+r',
      send: 'ctrl+space',
      forward: 'ctrl+f',
      drop: 'ctrl+d',
      'toggle-intercept': 'ctrl+i',
      'decode-selection': 'ctrl+shift+d',
      'tab-next': 'ctrl+shift+]',
      'tab-prev': 'ctrl+shift+[',
      'copy-as-curl': 'ctrl+shift+c',
      'copy-raw': 'ctrl+shift+s',
      'show-shortcuts': '?',
      'focus-search': '/',
      find: 'ctrl+f',
    });
    expect(ACTION_LABELS['focus-search']).toBe('Focus the History search');
    expect(ACTION_LABELS.find).toBe('Find in the message');
  });

  it('resolves events to chords', () => {
    expect(chordFromEvent(key({ key: 'r', code: 'KeyR', ctrlKey: true }))).toBe('ctrl+r');
    expect(chordFromEvent(key({ key: ' ', code: 'Space', ctrlKey: true }))).toBe('ctrl+space');
    expect(chordFromEvent(key({ key: '}', code: 'BracketRight', ctrlKey: true, shiftKey: true }))).toBe(
      'ctrl+shift+]',
    );
  });
});

describe('keymap dispatch', () => {
  it('fires the registered handler and prevents default for a bound chord', () => {
    const km = createKeymap();
    const h = vi.fn();
    km.register('forward', h);
    const ev = key({ key: 'f', code: 'KeyF', ctrlKey: true });
    km.dispatch(ev);
    expect(h).toHaveBeenCalledTimes(1);
    expect(ev.defaultPrevented).toBe(true);
  });

  it('is a no-op (browser default kept) for a bound chord with no handler', () => {
    const km = createKeymap();
    const ev = key({ key: 'd', code: 'KeyD', ctrlKey: true });
    expect(() => km.dispatch(ev)).not.toThrow();
    expect(ev.defaultPrevented).toBe(false);
  });

  it('is a no-op for an unbound chord', () => {
    const km = createKeymap();
    const h = vi.fn();
    for (const a of ACTIONS) km.register(a, h);
    const ev = key({ key: 'q', code: 'KeyQ', ctrlKey: true });
    expect(() => km.dispatch(ev)).not.toThrow();
    expect(h).not.toHaveBeenCalled();
    expect(ev.defaultPrevented).toBe(false);
  });

  it('does not treat Cmd as Ctrl', () => {
    const km = createKeymap();
    const h = vi.fn();
    km.register('forward', h);
    const ev = key({ key: 'f', code: 'KeyF', metaKey: true });
    km.dispatch(ev);
    expect(h).not.toHaveBeenCalled();
    expect(ev.defaultPrevented).toBe(false);
  });

  it('remap moves a binding: new chord fires, old one no longer', () => {
    const km = createKeymap();
    const h = vi.fn();
    km.register('drop', h);
    km.remap('drop', 'ctrl+shift+x');
    expect(km.chordFor('drop')).toBe('ctrl+shift+x');
    const fresh = key({ key: 'X', code: 'KeyX', ctrlKey: true, shiftKey: true });
    km.dispatch(fresh);
    expect(h).toHaveBeenCalledTimes(1);
    expect(fresh.defaultPrevented).toBe(true);
    const old = key({ key: 'd', code: 'KeyD', ctrlKey: true });
    km.dispatch(old);
    expect(h).toHaveBeenCalledTimes(1);
    expect(old.defaultPrevented).toBe(false);
  });

  it('remapping onto a taken chord steals it from the previous action', () => {
    const km = createKeymap();
    const f = vi.fn();
    const d = vi.fn();
    km.register('forward', f);
    km.register('drop', d);
    km.remap('drop', 'ctrl+f');
    km.dispatch(key({ key: 'f', code: 'KeyF', ctrlKey: true }));
    expect(d).toHaveBeenCalledTimes(1);
    expect(f).not.toHaveBeenCalled();
  });

  it('register returns an unregister function', () => {
    const km = createKeymap();
    const h = vi.fn();
    const off = km.register('send', h);
    off();
    km.dispatch(key({ key: ' ', code: 'Space', ctrlKey: true }));
    expect(h).not.toHaveBeenCalled();
  });
});

describe('keymap bindings()', () => {
  it('lists every default action with a non-empty chord and label', () => {
    const km = createKeymap();
    const rows = km.bindings();
    expect(rows.map((r) => r.action).sort()).toEqual([...ACTIONS].sort());
    for (const r of rows) {
      expect(r.chord?.length).toBeGreaterThan(0);
      expect(r.label.trim().length).toBeGreaterThan(0);
      expect(r.chord).toBe(DEFAULT_BINDINGS[r.action]);
    }
  });

  it('reflects a remap', () => {
    const km = createKeymap();
    km.remap('drop', 'ctrl+shift+x');
    expect(km.bindings().find((r) => r.action === 'drop')?.chord).toBe('ctrl+shift+x');
  });

  it('formats chords for display', () => {
    expect(formatChord('ctrl+shift+d')).toBe('Ctrl+Shift+D');
    expect(formatChord('ctrl+space')).toBe('Ctrl+Space');
    expect(formatChord('?')).toBe('?');
    expect(formatChord('ctrl+shift+]')).toBe('Ctrl+Shift+]');
  });
});

describe('plain-key chords (no Ctrl/Alt)', () => {
  it('resolves ? with or without Shift to the same chord', () => {
    expect(chordFromEvent(key({ key: '?', code: 'Slash', shiftKey: true }))).toBe('?');
    expect(chordFromEvent(key({ key: '?', code: 'Slash' }))).toBe('?');
  });

  it('fires ? outside editable targets only', () => {
    const km = createKeymap();
    const h = vi.fn();
    km.register('show-shortcuts', h);
    const onBody = key({ key: '?', code: 'Slash', shiftKey: true });
    document.body.dispatchEvent(onBody);
    km.dispatch(onBody);
    expect(h).toHaveBeenCalledTimes(1);

    for (const tag of ['input', 'textarea', 'select']) {
      const el = document.createElement(tag);
      document.body.append(el);
      const ev = key({ key: '?', code: 'Slash', shiftKey: true });
      el.dispatchEvent(ev);
      km.dispatch(ev);
      expect(ev.defaultPrevented).toBe(false);
      el.remove();
    }
    const ce = document.createElement('div');
    ce.setAttribute('contenteditable', 'true'); // jsdom does not reflect .contentEditable
    document.body.append(ce);
    const ev = key({ key: '?', code: 'Slash', shiftKey: true });
    ce.dispatchEvent(ev);
    km.dispatch(ev);
    expect(ev.defaultPrevented).toBe(false);
    ce.remove();
    expect(h).toHaveBeenCalledTimes(1);
  });

  it('still fires Ctrl chords inside an input', () => {
    const km = createKeymap();
    const h = vi.fn();
    km.register('forward', h);
    const el = document.createElement('input');
    document.body.append(el);
    const ev = key({ key: 'f', code: 'KeyF', ctrlKey: true });
    el.dispatchEvent(ev);
    km.dispatch(ev);
    expect(h).toHaveBeenCalledTimes(1);
    el.remove();
  });
});

describe('normalizeChord', () => {
  it('orders modifiers, lower-cases, maps space, keeps brackets', () => {
    expect(normalizeChord('Shift+Ctrl+E')).toBe('ctrl+shift+e');
    expect(normalizeChord('alt+ctrl+shift+X')).toBe('ctrl+alt+shift+x');
    expect(normalizeChord('Ctrl+Space')).toBe('ctrl+space');
    expect(normalizeChord('ctrl+shift+]')).toBe('ctrl+shift+]');
    expect(normalizeChord('?')).toBe('?');
    expect(normalizeChord('ctrl++')).toBe('ctrl++');
  });
  it('rejects empty, lone-modifier and unknown-modifier chords', () => {
    expect(normalizeChord('')).toBeNull();
    expect(normalizeChord('ctrl+shift')).toBeNull();
    expect(normalizeChord('hyper+e')).toBeNull();
  });
});

describe('keymap overrides (persisted)', () => {
  const KEY = 'test.keymap';
  afterEach(() => localStorage.clear());

  it('round-trips remap through storage and reset removes it', () => {
    const a = createKeymap(DEFAULT_BINDINGS, { storageKey: KEY });
    a.remap('send-to-repeater', 'Ctrl+Shift+E');
    expect(JSON.parse(localStorage.getItem(KEY) ?? '{}')).toEqual({
      'send-to-repeater': 'ctrl+shift+e',
    });
    const b = createKeymap(DEFAULT_BINDINGS, { storageKey: KEY });
    expect(b.chordFor('send-to-repeater')).toBe('ctrl+shift+e');
    expect(b.isOverridden('send-to-repeater')).toBe(true);
    expect(b.bindings().find((x) => x.action === 'send-to-repeater')?.chord).toBe('ctrl+shift+e');
    b.reset('send-to-repeater');
    expect(b.chordFor('send-to-repeater')).toBe('ctrl+r');
    expect(localStorage.getItem(KEY)).toBeNull();
  });

  it('the old chord is inert after remap, and the new one fires', () => {
    const km = createKeymap(DEFAULT_BINDINGS, { storageKey: KEY });
    const h = vi.fn();
    km.register('send-to-repeater', h);
    km.remap('send-to-repeater', 'ctrl+shift+e');
    km.dispatch(key({ key: 'r', code: 'KeyR', ctrlKey: true }));
    expect(h).not.toHaveBeenCalled();
    km.dispatch(key({ key: 'E', code: 'KeyE', ctrlKey: true, shiftKey: true }));
    expect(h).toHaveBeenCalledTimes(1);
  });

  it('reset() with no argument clears every override', () => {
    const km = createKeymap(DEFAULT_BINDINGS, { storageKey: KEY });
    km.remap('drop', 'ctrl+shift+x');
    km.remap('forward', 'ctrl+shift+y');
    km.reset();
    expect(km.bindings().map((b) => [b.action, b.chord])).toEqual(
      ACTIONS.map((a) => [a, DEFAULT_BINDINGS[a]]),
    );
    expect(localStorage.getItem(KEY)).toBeNull();
  });

  it('ignores corrupt JSON, unknown actions, invalid and reserved chords on load', () => {
    localStorage.setItem(KEY, '{not json');
    expect(createKeymap(DEFAULT_BINDINGS, { storageKey: KEY }).chordFor('drop')).toBe('ctrl+d');
    localStorage.setItem(
      KEY,
      JSON.stringify({
        bogus: 'ctrl+q',
        drop: 'ctrl+t',
        forward: 'x',
        send: 42,
        'tab-next': 'ctrl+shift+x',
        'copy-raw': 'ctrl+d',
      }),
    );
    const km = createKeymap(DEFAULT_BINDINGS, { storageKey: KEY });
    expect(km.chordFor('drop')).toBe('ctrl+d');
    expect(km.chordFor('forward')).toBe('ctrl+f');
    expect(km.chordFor('send')).toBe('ctrl+space');
    expect(km.chordFor('copy-raw')).toBe('ctrl+shift+s');
    expect(km.chordFor('tab-next')).toBe('ctrl+shift+x');
  });

  it('ignores bare special keys on load', () => {
    localStorage.setItem(KEY, JSON.stringify({ drop: 'tab', send: 'enter', forward: 'f1', copy: 'x', 'tab-next': 'shift+tab', 'tab-prev': 'space', 'copy-raw': 'escape' }));
    const km = createKeymap(DEFAULT_BINDINGS, { storageKey: KEY });
    expect(km.bindings().map((b) => b.chord)).toEqual(ACTIONS.map((a) => DEFAULT_BINDINGS[a]));
  });

  it('load is independent of storage order (swap onto a vacated default)', () => {
    localStorage.setItem(KEY, JSON.stringify({ send: 'ctrl+shift+e', drop: 'ctrl+space' }));
    const km = createKeymap(DEFAULT_BINDINGS, { storageKey: KEY });
    expect(km.chordFor('drop')).toBe('ctrl+space');
    expect(km.chordFor('send')).toBe('ctrl+shift+e');
  });

  it('reset cascades when a restored default is held by another override', () => {
    const km = createKeymap(DEFAULT_BINDINGS, { storageKey: KEY });
    km.remap('send-to-repeater', 'ctrl+shift+e');
    km.remap('drop', 'ctrl+r'); // ctrl+r is free now, but drop vacated ctrl+d
    km.reset('send-to-repeater');
    expect(km.chordFor('send-to-repeater')).toBe('ctrl+r');
    expect(km.chordFor('drop')).toBe('ctrl+d');
  });

  it('notifies subscribers on remap and reset', () => {
    const km = createKeymap();
    const fn = vi.fn();
    const off = km.subscribe(fn);
    km.remap('drop', 'ctrl+shift+x');
    km.reset('drop');
    off();
    km.reset();
    expect(fn).toHaveBeenCalledTimes(2);
  });
});

describe('validateChord', () => {
  const km = createKeymap();
  it('refuses browser-reserved chords', () => {
    for (const c of ['ctrl+t', 'ctrl+n', 'ctrl+w', 'ctrl+shift+t', 'ctrl+shift+n', 'ctrl+shift+w']) {
      const r = km.validateChord(c, 'drop');
      expect(r.ok).toBe(false);
      if (!r.ok) expect(r.reason).toMatch(/reserved/);
    }
  });
  it('refuses a bare letter or digit but allows modified keys', () => {
    expect(km.validateChord('e', 'drop').ok).toBe(false);
    expect(km.validateChord('shift+e', 'drop').ok).toBe(false);
    expect(km.validateChord('7', 'drop').ok).toBe(false);
    expect(km.validateChord('ctrl+e', 'drop').ok).toBe(true);
    expect(km.validateChord('alt+7', 'drop').ok).toBe(true);
  });
  it('refuses bare special keys', () => {
    for (const c of ['tab', 'enter', 'space', 'escape', 'f1', 'shift+tab', 'backspace', 'arrowup']) {
      expect(km.validateChord(c, 'drop').ok).toBe(false);
    }
    expect(km.validateChord('?', 'show-shortcuts').ok).toBe(true);
    expect(km.validateChord('ctrl+tab', 'drop').ok).toBe(true);
  });
  it('refuses a conflict naming the other action, allows its own chord', () => {
    const r = km.validateChord('Ctrl+F', 'drop');
    expect(r.ok).toBe(false);
    if (!r.ok) expect(r.reason).toContain('Forward the held request');
    expect(km.validateChord('ctrl+d', 'drop').ok).toBe(true);
  });
  it('refuses garbage', () => {
    expect(km.validateChord('ctrl+shift', 'drop').ok).toBe(false);
  });
});

describe('recordChord', () => {
  const km = createKeymap();
  it('ignores lone modifiers, cancels on Escape, accepts a valid chord', () => {
    expect(recordChord(key({ key: 'Control', ctrlKey: true }), 'drop', km).kind).toBe('ignore');
    expect(recordChord(key({ key: 'Escape', code: 'Escape' }), 'drop', km).kind).toBe('cancel');
    expect(recordChord(key({ key: 'E', code: 'KeyE', ctrlKey: true, shiftKey: true }), 'drop', km)).toEqual({
      kind: 'ok',
      chord: 'ctrl+shift+e',
    });
  });
  it('refuses Ctrl+T', () => {
    expect(recordChord(key({ key: 't', code: 'KeyT', ctrlKey: true }), 'drop', km).kind).toBe('refused');
  });
});

// ---- V1: scoped keymap ------------------------------------------------------

const ctrlF = () => key({ key: 'f', code: 'KeyF', ctrlKey: true });

/** A `data-keyscope` element appended to `parent` (document.body by default). */
function scopeEl(scope: string, parent: Element = document.body): HTMLElement {
  const el = document.createElement('div');
  el.setAttribute('data-keyscope', scope);
  parent.append(el);
  return el;
}

/** Dispatch `ev` from `target` (so it carries a real target) through `km`. */
function fire(
  km: ReturnType<typeof createKeymap>,
  target: Element,
  ev: KeyboardEvent,
  route?: Scope | null,
): KeyboardEvent {
  target.addEventListener('keydown', (e) => km.dispatch(e as KeyboardEvent, route), { once: true });
  target.dispatchEvent(ev);
  return ev;
}

describe('keymap scopes', () => {
  afterEach(() => {
    document.body.replaceChildren();
  });

  it('lists the scopes in display order with labels', () => {
    expect([...SCOPES]).toEqual(['global', 'history', 'repeater', 'intercept', 'viewer']);
    expect(SCOPE_LABELS).toEqual({
      global: 'Everywhere',
      history: 'History',
      repeater: 'Repeater',
      intercept: 'Intercept',
      viewer: 'Message viewer',
    });
    expect(PARENTS.viewer).toEqual(['history', 'repeater', 'global']);
  });

  it('gives every action a scope', () => {
    expect(ACTION_SCOPES).toEqual({
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
    });
    for (const a of ACTIONS) expect(SCOPES).toContain(ACTION_SCOPES[a]);
  });

  it('coActive: nested and global scopes overlap, sibling views do not', () => {
    expect(coActive('viewer', 'history')).toBe(true);
    expect(coActive('history', 'viewer')).toBe(true);
    expect(coActive('viewer', 'repeater')).toBe(true);
    expect(coActive('viewer', 'intercept')).toBe(false);
    expect(coActive('intercept', 'viewer')).toBe(false);
    expect(coActive('history', 'repeater')).toBe(false);
    expect(coActive('history', 'intercept')).toBe(false);
    expect(coActive('repeater', 'intercept')).toBe(false);
    for (const s of SCOPES) {
      expect(coActive('global', s)).toBe(true);
      expect(coActive(s, 'global')).toBe(true);
      expect(coActive(s, s)).toBe(true);
    }
  });

  it('the defaults have no co-active clash (every pair enumerated)', () => {
    const clashes: string[] = [];
    for (const a of ACTIONS) {
      for (const b of ACTIONS) {
        if (a < b && DEFAULT_BINDINGS[a] === DEFAULT_BINDINGS[b] && coActive(ACTION_SCOPES[a], ACTION_SCOPES[b])) {
          clashes.push(`${a} / ${b}`);
        }
      }
    }
    expect(clashes).toEqual([]);
  });

  it('maps router paths to route scopes', () => {
    expect(routeScopeFor('/')).toBe('history');
    expect(routeScopeFor('/repeater')).toBe('repeater');
    expect(routeScopeFor('/intercept')).toBe('intercept');
    expect(routeScopeFor('/settings')).toBeNull();
    expect(routeScopeFor('/decoder')).toBeNull();
  });

  it('scopeChain: DOM scopes innermost first, then the route, then global', () => {
    const history = scopeEl('history');
    const viewer = scopeEl('viewer', history);
    const inner = document.createElement('button');
    viewer.append(inner);
    expect(scopeChain(inner, 'history')).toEqual(['viewer', 'history', 'global']);
    expect(scopeChain(inner, null)).toEqual(['viewer', 'history', 'global']);
    expect(scopeChain(document.body, 'intercept')).toEqual(['intercept', 'global']);
    expect(scopeChain(document.body, null)).toEqual(['global']);
    expect(scopeChain(null, 'repeater')).toEqual(['repeater', 'global']);
    // Unknown scope names are ignored.
    const bogus = scopeEl('nonsense');
    expect(scopeChain(bogus, null)).toEqual(['global']);
  });
});

describe('keymap scoped dispatch', () => {
  afterEach(() => {
    document.body.replaceChildren();
  });

  it('Find (viewer, bound to viewer A) and Forward (intercept) share Ctrl+F', () => {
    const km = createKeymap();
    const viewerA = scopeEl('viewer', scopeEl('history'));
    const find = vi.fn();
    const forward = vi.fn();
    km.register('find', find, { el: viewerA });
    km.register('forward', forward);

    const inA = fire(km, viewerA, ctrlF(), 'history');
    expect(find).toHaveBeenCalledTimes(1);
    expect(forward).not.toHaveBeenCalled();
    expect(inA.defaultPrevented).toBe(true);

    const outside = fire(km, document.body, ctrlF(), 'intercept');
    expect(forward).toHaveBeenCalledTimes(1);
    expect(find).toHaveBeenCalledTimes(1);
    expect(outside.defaultPrevented).toBe(true);
  });

  it('picks the element-bound handler whose element contains the target', () => {
    const km = createKeymap();
    const history = scopeEl('history');
    const a = scopeEl('viewer', history);
    const b = scopeEl('viewer', history);
    const inB = document.createElement('span');
    b.append(inB);
    const ha = vi.fn();
    const hb = vi.fn();
    km.register('find', ha, { el: a });
    const offB = km.register('find', hb, { el: b });
    fire(km, inB, ctrlF(), 'history');
    expect(hb).toHaveBeenCalledTimes(1);
    expect(ha).not.toHaveBeenCalled();
    fire(km, a, ctrlF(), 'history');
    expect(ha).toHaveBeenCalledTimes(1);
    // Unregistering B leaves no handler for B's subtree: the key falls through.
    offB();
    const ev = fire(km, inB, ctrlF(), 'history');
    expect(hb).toHaveBeenCalledTimes(1);
    expect(ev.defaultPrevented).toBe(false);
  });

  it('a declining handler (returns false) stops dispatch without preventDefault', () => {
    localStorage.setItem('scoped.decline', JSON.stringify({ forward: 'ctrl+alt+f', 'toggle-intercept': 'ctrl+f' }));
    const km = createKeymap(DEFAULT_BINDINGS, { storageKey: 'scoped.decline' });
    localStorage.clear();
    const viewer = scopeEl('viewer', scopeEl('history'));
    const find = vi.fn(() => false);
    const toggle = vi.fn();
    km.register('find', find, { el: viewer });
    km.register('toggle-intercept', toggle);
    const ev = fire(km, viewer, ctrlF(), 'history');
    expect(find).toHaveBeenCalledTimes(1);
    expect(toggle).not.toHaveBeenCalled();
    expect(ev.defaultPrevented).toBe(false);
  });

  it('a scope with no handler for the chord passes it outwards', () => {
    const km = createKeymap();
    km.remap('send-to-repeater', 'ctrl+e');
    const viewer = scopeEl('viewer', scopeEl('history'));
    const s = vi.fn();
    km.register('send-to-repeater', s);
    fire(km, viewer, key({ key: 'e', code: 'KeyE', ctrlKey: true }), 'history');
    expect(s).toHaveBeenCalledTimes(1);
  });

  it('ignores plain-key chords inside a menu or dialog, not Ctrl chords', () => {
    const km = createKeymap();
    const show = vi.fn();
    const send = vi.fn();
    km.register('show-shortcuts', show);
    km.register('send-to-repeater', send);
    for (const role of ['menu', 'dialog']) {
      const box = document.createElement('div');
      box.setAttribute('role', role);
      const item = document.createElement('button');
      box.append(item);
      document.body.append(box);
      const ev = fire(km, item, key({ key: '?', code: 'Slash', shiftKey: true }), 'history');
      expect(ev.defaultPrevented).toBe(false);
      fire(km, item, key({ key: 'r', code: 'KeyR', ctrlKey: true }), 'history');
    }
    expect(show).not.toHaveBeenCalled();
    expect(send).toHaveBeenCalledTimes(2);
  });

  it('without a route scope, every U2 action still dispatches (pre-migration callers)', () => {
    const km = createKeymap();
    const forward = vi.fn();
    const send = vi.fn();
    km.register('forward', forward);
    km.register('send', send);
    fire(km, document.body, ctrlF());
    fire(km, document.body, key({ key: ' ', code: 'Space', ctrlKey: true }));
    expect(forward).toHaveBeenCalledTimes(1);
    expect(send).toHaveBeenCalledTimes(1);
  });

  it('with an explicit null route, view-scoped actions do not fire', () => {
    const km = createKeymap();
    const forward = vi.fn();
    km.register('forward', forward);
    const ev = fire(km, document.body, ctrlF(), null);
    expect(forward).not.toHaveBeenCalled();
    expect(ev.defaultPrevented).toBe(false);
  });
});

describe('keymap per-scope conflicts', () => {
  it('validateChord accepts a chord held only in a disjoint scope', () => {
    const km = createKeymap();
    expect(km.validateChord('ctrl+f', 'find').ok).toBe(true);
    km.remap('find', 'ctrl+shift+f');
    // Back onto Forward's chord: disjoint from the viewer, so accepted.
    expect(km.validateChord('ctrl+f', 'find').ok).toBe(true);
    // Repeater's send onto Intercept's drop: disjoint.
    expect(km.validateChord('ctrl+d', 'send').ok).toBe(true);
  });

  it('validateChord refuses a co-active holder, naming it and its scope', () => {
    const km = createKeymap();
    const r = km.validateChord('ctrl+r', 'copy-raw');
    expect(r).toEqual({ ok: false, reason: 'Ctrl+R is already used by "Send to Repeater" (Everywhere).' });
    // A global chord is co-active with every scope; the first holder in
    // ACTIONS order is named.
    const f = km.validateChord('ctrl+f', 'copy-raw');
    expect(f).toEqual({
      ok: false,
      reason: 'Ctrl+F is already used by "Forward the held request (Intercept)" (Intercept).',
    });
    const v = km.validateChord('/', 'find');
    expect(v.ok).toBe(false);
    if (!v.ok) expect(v.reason).toBe('/ is already used by "Focus the History search" (History).');
  });

  it('remap steals only from co-active holders', () => {
    const km = createKeymap();
    km.remap('send', 'ctrl+d'); // repeater vs intercept: both keep it
    expect(km.chordFor('send')).toBe('ctrl+d');
    expect(km.chordFor('drop')).toBe('ctrl+d');
    km.remap('copy-raw', 'ctrl+f'); // global: takes it from Forward AND Find
    expect(km.chordFor('copy-raw')).toBe('ctrl+f');
    expect(km.chordFor('forward')).toBeUndefined();
    expect(km.chordFor('find')).toBeUndefined();
    const rows = km.bindings();
    expect(rows.map((r) => r.action)).toEqual([...ACTIONS]);
    expect(rows.find((r) => r.action === 'forward')).toEqual({
      action: 'forward',
      chord: null,
      label: 'Forward the held request (Intercept)',
      scope: 'intercept',
    });
    expect(rows.find((r) => r.action === 'find')?.chord).toBeNull();
  });

  it('reset cascades only over co-active holders', () => {
    const km = createKeymap();
    km.remap('find', 'ctrl+shift+f');
    km.remap('drop', 'ctrl+alt+d');
    km.remap('send', 'ctrl+alt+d'); // disjoint from drop: both hold it
    km.reset('find');
    expect(km.chordFor('find')).toBe('ctrl+f');
    expect(km.chordFor('forward')).toBe('ctrl+f');
    km.reset('drop');
    expect(km.chordFor('drop')).toBe('ctrl+d');
    expect(km.chordFor('send')).toBe('ctrl+alt+d');
    km.reset();
    expect(km.bindings().map((b) => [b.action, b.chord])).toEqual(ACTIONS.map((a) => [a, DEFAULT_BINDINGS[a]]));
  });

  it('reset of an action already at its default leaves a shadowed override alone', () => {
    localStorage.setItem(
      'test.keymap.reset-guard',
      JSON.stringify({ forward: 'ctrl+alt+f', 'toggle-intercept': 'ctrl+f' }),
    );
    const km = createKeymap(DEFAULT_BINDINGS, { storageKey: 'test.keymap.reset-guard' });
    expect(km.shadows('find')).toEqual(['toggle-intercept']);
    km.reset('find');
    expect(km.chordFor('find')).toBe('ctrl+f');
    expect(km.chordFor('toggle-intercept')).toBe('ctrl+f');
    expect(km.isOverridden('toggle-intercept')).toBe(true);
    localStorage.removeItem('test.keymap.reset-guard');
  });

  it('shadows() is empty for the defaults', () => {
    const km = createKeymap();
    for (const a of ACTIONS) expect(km.shadows(a)).toEqual([]);
  });
});

describe('keymap U2 storage migration', () => {
  const KEY = 'test.keymap.u2';
  afterEach(() => {
    localStorage.clear();
    document.body.replaceChildren();
  });

  it('loads {forward: ctrl+alt+f, toggle-intercept: ctrl+f} verbatim; Find shadows the toggle in the viewer', () => {
    localStorage.setItem(KEY, JSON.stringify({ forward: 'ctrl+alt+f', 'toggle-intercept': 'ctrl+f' }));
    const km = createKeymap(DEFAULT_BINDINGS, { storageKey: KEY });
    expect(km.chordFor('forward')).toBe('ctrl+alt+f');
    expect(km.chordFor('toggle-intercept')).toBe('ctrl+f');
    expect(km.chordFor('find')).toBe('ctrl+f');
    expect(km.shadows('find')).toEqual(['toggle-intercept']);
    expect(km.shadows('toggle-intercept')).toEqual([]);
    // Nothing is rewritten on load.
    expect(JSON.parse(localStorage.getItem(KEY) ?? '{}')).toEqual({
      forward: 'ctrl+alt+f',
      'toggle-intercept': 'ctrl+f',
    });

    const viewer = scopeEl('viewer', scopeEl('history'));
    const find = vi.fn();
    const toggle = vi.fn();
    km.register('find', find, { el: viewer });
    km.register('toggle-intercept', toggle);
    fire(km, viewer, ctrlF(), 'history');
    expect(find).toHaveBeenCalledTimes(1);
    expect(toggle).not.toHaveBeenCalled();
    fire(km, document.body, ctrlF(), 'history');
    expect(toggle).toHaveBeenCalledTimes(1);
    expect(find).toHaveBeenCalledTimes(1);
  });

  it('loads {copy-raw: "/"}; focus-search shadows it inside History', () => {
    localStorage.setItem(KEY, JSON.stringify({ 'copy-raw': '/' }));
    const km = createKeymap(DEFAULT_BINDINGS, { storageKey: KEY });
    expect(km.chordFor('copy-raw')).toBe('/');
    expect(km.chordFor('focus-search')).toBe('/');
    expect(km.shadows('focus-search')).toEqual(['copy-raw']);

    const history = scopeEl('history');
    const search = vi.fn();
    const copy = vi.fn();
    km.register('focus-search', search);
    km.register('copy-raw', copy);
    const slash = () => key({ key: '/', code: 'Slash' });
    fire(km, history, slash(), 'history');
    expect(search).toHaveBeenCalledTimes(1);
    expect(copy).not.toHaveBeenCalled();
    fire(km, document.body, slash(), 'repeater');
    expect(copy).toHaveBeenCalledTimes(1);
  });

  it('still drops a co-active clash with a U2 default (copy-raw onto Drop)', () => {
    localStorage.setItem(KEY, JSON.stringify({ 'copy-raw': 'ctrl+d' }));
    const km = createKeymap(DEFAULT_BINDINGS, { storageKey: KEY });
    expect(km.chordFor('copy-raw')).toBe('ctrl+shift+s');
    expect(km.chordFor('drop')).toBe('ctrl+d');
  });

  it('keeps an override that only clashes in a disjoint scope', () => {
    localStorage.setItem(KEY, JSON.stringify({ send: 'ctrl+d' }));
    const km = createKeymap(DEFAULT_BINDINGS, { storageKey: KEY });
    expect(km.chordFor('send')).toBe('ctrl+d');
    expect(km.chordFor('drop')).toBe('ctrl+d');
    expect(km.shadows('send')).toEqual([]);
  });

  it('a V1 find override round-trips', () => {
    const a = createKeymap(DEFAULT_BINDINGS, { storageKey: KEY });
    a.remap('find', 'ctrl+shift+f');
    const b = createKeymap(DEFAULT_BINDINGS, { storageKey: KEY });
    expect(b.chordFor('find')).toBe('ctrl+shift+f');
    expect(b.isOverridden('find')).toBe(true);
  });
});
