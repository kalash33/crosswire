import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import Toast from './Toast.svelte';
import { toasts } from './toasts.svelte';

describe('toasts', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    toasts.clear();
  });
  afterEach(() => {
    toasts.clear();
    vi.useRealTimers();
  });

  it('push adds a toast that auto-dismisses after the default 4 s', () => {
    toasts.push({ kind: 'success', text: 'Copied URL' });
    expect(toasts.items).toHaveLength(1);
    expect(toasts.items[0]).toMatchObject({ kind: 'success', text: 'Copied URL' });
    vi.advanceTimersByTime(3999);
    expect(toasts.items).toHaveLength(1);
    vi.advanceTimersByTime(1);
    expect(toasts.items).toHaveLength(0);
  });

  it('honours a custom ttl', () => {
    toasts.push({ kind: 'error', text: 'Send failed', ttlMs: 500 });
    vi.advanceTimersByTime(499);
    expect(toasts.items).toHaveLength(1);
    vi.advanceTimersByTime(1);
    expect(toasts.items).toHaveLength(0);
  });

  it('dedupes identical consecutive text and restarts its timer', () => {
    const a = toasts.push({ kind: 'info', text: 'Disconnected' });
    vi.advanceTimersByTime(3000);
    const b = toasts.push({ kind: 'info', text: 'Disconnected' });
    expect(b).toBe(a);
    expect(toasts.items).toHaveLength(1);
    vi.advanceTimersByTime(3000);
    expect(toasts.items).toHaveLength(1);
    vi.advanceTimersByTime(1000);
    expect(toasts.items).toHaveLength(0);
  });

  it('keeps different or non-consecutive texts as separate toasts', () => {
    toasts.push({ kind: 'info', text: 'A' });
    toasts.push({ kind: 'info', text: 'B' });
    toasts.push({ kind: 'info', text: 'A' });
    expect(toasts.items.map((t) => t.text)).toEqual(['A', 'B', 'A']);
  });

  it('dismiss removes one toast and cancels its timer', () => {
    const id = toasts.push({ kind: 'info', text: 'X' });
    toasts.push({ kind: 'info', text: 'Y' });
    toasts.dismiss(id);
    expect(toasts.items.map((t) => t.text)).toEqual(['Y']);
    vi.advanceTimersByTime(4000);
    expect(toasts.items).toHaveLength(0);
  });
});

describe('Toast outlet', () => {
  let cleanup: (() => void) | undefined;
  afterEach(() => {
    cleanup?.();
    cleanup = undefined;
    toasts.clear();
    document.body.replaceChildren();
  });

  it('announces errors with role="alert" and others in a polite status region', () => {
    const c = mount(Toast, { target: document.body });
    cleanup = () => unmount(c);
    toasts.push({ kind: 'info', text: 'Copied URL' });
    toasts.push({ kind: 'error', text: '<b>Send failed</b>' });
    flushSync();
    const alerts = document.querySelectorAll('[role="alert"]');
    expect(alerts).toHaveLength(1);
    expect(alerts[0].textContent).toContain('<b>Send failed</b>');
    expect(document.querySelector('b')).toBeNull();
    const status = document.querySelector('[role="status"]')!;
    expect(status.getAttribute('aria-live')).toBe('polite');
    expect(status.textContent).toContain('Copied URL');
    expect(status.textContent).not.toContain('Send failed');
  });
});
