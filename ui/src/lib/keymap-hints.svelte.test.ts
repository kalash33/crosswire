// Runes need the Svelte compiler: the `.svelte.` infix makes vite-plugin-svelte
// compile this test, so `$effect.root` and `$derived` work as in a component.
import { afterEach, describe, expect, it } from 'vitest';
import { flushSync } from 'svelte';
import { keymap } from './keymap';
import { chordLabel, findHint } from './keymap-hints.svelte';

describe('keymap hints', () => {
  afterEach(() => {
    keymap.reset();
    localStorage.clear();
  });

  it('formats the current chord, undefined when unbound', () => {
    expect(chordLabel('send')).toBe('Ctrl+Space');
    expect(chordLabel('decode-selection')).toBe('Ctrl+Shift+D');
    expect(findHint()).toBe('Ctrl+F in the viewer');
    keymap.remap('copy-raw', 'ctrl+f'); // global: takes Find's chord
    expect(chordLabel('find')).toBeUndefined();
    expect(findHint()).toBeUndefined();
  });

  it('a $derived over chordLabel updates after remap and reset', () => {
    let send: string | undefined;
    let find: string | undefined;
    const stop = $effect.root(() => {
      const label = $derived(chordLabel('send'));
      const hint = $derived(findHint());
      $effect(() => {
        send = label;
        find = hint;
      });
    });
    try {
      flushSync();
      expect(send).toBe('Ctrl+Space');
      expect(find).toBe('Ctrl+F in the viewer');
      keymap.remap('send', 'ctrl+alt+s');
      keymap.remap('find', 'ctrl+shift+f');
      flushSync();
      expect(send).toBe('Ctrl+Alt+S');
      expect(find).toBe('Ctrl+Shift+F in the viewer');
      keymap.reset();
      flushSync();
      expect(send).toBe('Ctrl+Space');
      expect(find).toBe('Ctrl+F in the viewer');
    } finally {
      stop();
    }
  });
});
