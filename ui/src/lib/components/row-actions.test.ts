import { describe, expect, it, vi } from 'vitest';
import { actionTarget, rowMenuItems } from './row-actions';

const rows = [{ id: 'A' }, { id: 'B' }, { id: 'C' }];

describe('actionTarget', () => {
  it('prefers the focused row over the opened one, and says so', () => {
    expect(actionTarget('C', 'A', rows)).toEqual({ id: 'C', differs: true, index: 3 });
  });
  it('is the opened row when nothing else is focused', () => {
    expect(actionTarget(null, 'B', rows)).toEqual({ id: 'B', differs: false, index: 2 });
    expect(actionTarget('B', 'B', rows)).toEqual({ id: 'B', differs: false, index: 2 });
  });
  it('a focused row with nothing open differs; none at all is null', () => {
    expect(actionTarget('A', null, rows)).toEqual({ id: 'A', differs: true, index: 1 });
    expect(actionTarget(null, null, rows)).toEqual({ id: null, differs: false, index: 0 });
    expect(actionTarget('Z', null, rows).index).toBe(0);
  });
});

describe('rowMenuItems', () => {
  const a = { open: vi.fn(), send: vi.fn(), tag: vi.fn(), addHost: vi.fn(), preview: vi.fn() };
  const ids = (items: { testid?: string }[]) => items.map((i) => i.testid);

  it('omits Copy URL for an asterisk-form target and a null host', () => {
    expect(ids(rowMenuItems('X', { host: 'a.test', path: '*', status: 200 }, a))).not.toContain('copy-url');
    expect(ids(rowMenuItems('X', { host: null, path: '/', status: 200 }, a))).not.toContain('copy-url');
    expect(ids(rowMenuItems('X', { host: 'a.test', path: '/', status: 200 }, a))).toContain('copy-url');
  });

  it('disables Preview without a response and Add host without a host', () => {
    const items = rowMenuItems('X', { host: null, path: '/', status: null }, a);
    expect(items.find((i) => i.testid === 'row-menu-preview')?.disabled).toBe(true);
    expect(items.find((i) => i.testid === 'row-menu-add-host')?.disabled).toBe(true);
    const ok = rowMenuItems('X', { host: 'h', path: '/', status: 204 }, a);
    expect(ok.find((i) => i.testid === 'row-menu-preview')?.disabled).toBe(false);
  });
});
