import { describe, expect, it } from 'vitest';
import { tagSavePending, tagSavesSettled, trackTagSave } from './tag-saves';

describe('tag-saves', () => {
  it('tracks an in-flight save until it settles, and waits for a chained one', async () => {
    let r1: () => void = () => {};
    let r2: () => void = () => {};
    const p1 = new Promise<void>((r) => (r1 = r));
    const p2 = new Promise<void>((r) => (r2 = r));
    trackTagSave('X', p1);
    expect(tagSavePending('X')).toBe(true);
    let done = false;
    const wait = tagSavesSettled('X').then(() => (done = true));
    trackTagSave('X', p2);
    r1();
    await p1;
    await Promise.resolve();
    expect(done).toBe(false);
    r2();
    await wait;
    expect(done).toBe(true);
    expect(tagSavePending('X')).toBe(false);
  });

  it('settles immediately when nothing is in flight, and survives a rejected save', async () => {
    await expect(tagSavesSettled('none')).resolves.toBeUndefined();
    const bad = Promise.reject(new Error('x'));
    trackTagSave('Y', bad);
    await expect(tagSavesSettled('Y')).resolves.toBeUndefined();
    expect(tagSavePending('Y')).toBe(false);
  });
});
