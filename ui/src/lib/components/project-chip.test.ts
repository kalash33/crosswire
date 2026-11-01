import { afterEach, describe, expect, it } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import ProjectChip from './ProjectChip.svelte';

const XSS = '<img src=x onerror=alert(1)>';

describe('ProjectChip', () => {
  let cleanup: (() => void) | undefined;
  afterEach(() => {
    cleanup?.();
    cleanup = undefined;
    document.body.replaceChildren();
  });

  function render(name: string, path: string): HTMLElement {
    const target = document.createElement('div');
    document.body.append(target);
    const c = mount(ProjectChip, { target, props: { project: { name, path } } });
    flushSync();
    cleanup = () => unmount(c);
    return target;
  }

  it('shows the name with the path as its title', () => {
    const chip = render('acme', '/work/acme').querySelector('[data-testid="topbar-project"]');
    expect(chip?.textContent).toBe('acme');
    expect(chip?.getAttribute('title')).toBe('/work/acme');
  });

  it('renders a hostile name and path as text only', () => {
    const t = render(XSS, `/w/"${XSS}`);
    const chip = t.querySelector('[data-testid="topbar-project"]');
    expect(t.querySelector('img')).toBeNull();
    expect(chip?.children.length).toBe(0);
    expect(chip?.textContent).toBe(XSS);
    expect(chip?.getAttribute('title')).toBe(`/w/"${XSS}`);
  });
});
