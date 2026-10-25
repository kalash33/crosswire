import { describe, it, expect, afterEach } from 'vitest';
import { mount, unmount } from 'svelte';
import App from './App.svelte';

// Smoke test: the app must mount and expose a stable root element that later
// tests (Vitest components and Playwright e2e) can anchor on.
describe('App', () => {
  let cleanup: (() => void) | undefined;

  afterEach(() => {
    cleanup?.();
    cleanup = undefined;
    document.body.replaceChildren();
  });

  it('renders a root element with data-testid="app-root"', () => {
    const component = mount(App, { target: document.body });
    cleanup = () => unmount(component);

    const root = document.querySelector('[data-testid="app-root"]');
    expect(root).not.toBeNull();
  });
});
