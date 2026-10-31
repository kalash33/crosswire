import { describe, expect, it } from 'vitest';
import { createDecoderStore } from './decoder.svelte';

describe('decoder seed store', () => {
  it('hands a seed out exactly once', () => {
    const d = createDecoderStore();
    expect(d.consume()).toBeNull();
    d.seed(new Uint8Array([0x41, 0xe9]), false);
    const s = d.consume();
    expect(s && [...s.bytes]).toEqual([0x41, 0xe9]);
    expect(s?.nonLatin1).toBe(false);
    expect(d.consume()).toBeNull();
  });

  it('bumps the generation on every seed (a second chord reseeds)', () => {
    const d = createDecoderStore();
    const g0 = d.generation;
    d.seed(new Uint8Array([1]), false);
    d.seed(new Uint8Array([2]), true);
    expect(d.generation).toBe(g0 + 2);
    const s = d.consume();
    expect(s && [...s.bytes]).toEqual([2]);
    expect(s?.nonLatin1).toBe(true);
  });
});
