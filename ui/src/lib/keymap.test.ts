import { describe, expect, it, vi } from 'vitest';
import { ACTIONS, createKeymap, DEFAULT_BINDINGS, chordFromEvent } from './keymap';

function key(init: KeyboardEventInit): KeyboardEvent {
  return new KeyboardEvent('keydown', { cancelable: true, bubbles: true, ...init });
}

describe('keymap defaults (Burp-consistent)', () => {
  it('binds exactly the named actions to the chosen chords', () => {
    expect([...ACTIONS].sort()).toEqual(
      [
        'decode-selection',
        'drop',
        'forward',
        'send',
        'send-to-repeater',
        'tab-next',
        'tab-prev',
        'toggle-intercept',
      ].sort(),
    );
    expect(DEFAULT_BINDINGS).toEqual({
      'send-to-repeater': 'ctrl+r',
      send: 'ctrl+space',
      forward: 'ctrl+f',
      drop: 'ctrl+d',
      'toggle-intercept': 'ctrl+t',
      'decode-selection': 'ctrl+shift+d',
      'tab-next': 'ctrl+shift+]',
      'tab-prev': 'ctrl+shift+[',
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

  it('prevents default for a bound chord even with no handler', () => {
    const km = createKeymap();
    const ev = key({ key: 'd', code: 'KeyD', ctrlKey: true });
    expect(() => km.dispatch(ev)).not.toThrow();
    expect(ev.defaultPrevented).toBe(true);
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
