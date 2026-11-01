import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  ACTIONS,
  createKeymap,
  DEFAULT_BINDINGS,
  chordFromEvent,
  formatChord,
  normalizeChord,
  recordChord,
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
    });
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
      expect(r.chord.length).toBeGreaterThan(0);
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
