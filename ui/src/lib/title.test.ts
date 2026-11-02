import { describe, expect, it } from 'vitest';
import { windowTitle } from './title';

describe('windowTitle', () => {
  it('names the project, the view and the app', () => {
    expect(windowTitle('acme', 'History')).toBe('acme · History · Crosswire');
  });

  it('falls back to the app first when no project is named', () => {
    expect(windowTitle(null, 'Repeater')).toBe('Crosswire · Repeater');
    expect(windowTitle('', 'Repeater')).toBe('Crosswire · Repeater');
    expect(windowTitle('  ', 'Repeater')).toBe('Crosswire · Repeater');
  });

  it('drops an empty view', () => {
    expect(windowTitle('acme', '')).toBe('acme · Crosswire');
    expect(windowTitle(null, '')).toBe('Crosswire');
  });

  it('keeps a hostile name verbatim (document.title is text)', () => {
    const xss = '<img src=x onerror=alert(1)>';
    expect(windowTitle(xss, 'Scope')).toBe(`${xss} · Scope · Crosswire`);
    document.title = windowTitle(xss, 'Scope');
    expect(document.title).toBe(`${xss} · Scope · Crosswire`);
    expect(document.querySelector('img')).toBeNull();
  });
});
