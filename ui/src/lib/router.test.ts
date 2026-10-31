import { describe, expect, it } from 'vitest';
import { routes } from './router.svelte';

describe('route registry', () => {
  it('registers the primary views including Decoder and Settings', () => {
    expect(routes.map((r) => r.path)).toEqual([
      '/',
      '/repeater',
      '/intercept',
      '/scope',
      '/decoder',
      '/settings',
    ]);
    expect(routes.find((r) => r.path === '/decoder')?.label).toBe('Decoder');
    expect(routes.find((r) => r.path === '/settings')?.label).toBe('Settings');
  });
});
