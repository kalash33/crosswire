import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { THEME_KEY, applyStoredTheme, theme } from './theme';

describe('theme', () => {
  beforeEach(() => {
    localStorage.clear();
    delete document.documentElement.dataset.theme;
  });
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('defaults to dark when nothing is stored', () => {
    expect(applyStoredTheme()).toBe('dark');
    expect(theme.get()).toBe('dark');
    expect(document.documentElement.dataset.theme).toBe('dark');
  });

  it('applies a stored light theme', () => {
    localStorage.setItem(THEME_KEY, 'light');
    expect(applyStoredTheme()).toBe('light');
    expect(document.documentElement.dataset.theme).toBe('light');
  });

  it('ignores a garbage stored value', () => {
    localStorage.setItem(THEME_KEY, 'neon');
    expect(applyStoredTheme()).toBe('dark');
  });

  it('set applies and persists', () => {
    theme.set('light');
    expect(theme.get()).toBe('light');
    expect(document.documentElement.dataset.theme).toBe('light');
    expect(localStorage.getItem(THEME_KEY)).toBe('light');
  });

  it('toggle flips and persists', () => {
    applyStoredTheme();
    expect(theme.toggle()).toBe('light');
    expect(localStorage.getItem(THEME_KEY)).toBe('light');
    expect(theme.toggle()).toBe('dark');
    expect(localStorage.getItem(THEME_KEY)).toBe('dark');
    expect(document.documentElement.dataset.theme).toBe('dark');
  });

  it('a throwing storage never throws and still applies the theme', () => {
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('SecurityError');
    });
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('QuotaExceededError');
    });
    expect(() => applyStoredTheme()).not.toThrow();
    expect(theme.get()).toBe('dark');
    expect(() => theme.set('light')).not.toThrow();
    expect(document.documentElement.dataset.theme).toBe('light');
    expect(() => theme.toggle()).not.toThrow();
    expect(theme.get()).toBe('dark');
  });
});
