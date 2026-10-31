import { describe, expect, it } from 'vitest';
import { humanBytes, timeShort } from './format';

describe('humanBytes', () => {
  it('formats bytes, KB and MB with one decimal below 10 and none above', () => {
    expect(humanBytes(0)).toBe('0 B');
    expect(humanBytes(1)).toBe('1 B');
    expect(humanBytes(1023)).toBe('1023 B');
    expect(humanBytes(1024)).toBe('1.0 KB');
    expect(humanBytes(1843)).toBe('1.8 KB');
    expect(humanBytes(15 * 1024)).toBe('15 KB');
    expect(humanBytes(2.3 * 1024 * 1024)).toBe('2.3 MB');
    expect(humanBytes(512 * 1024 * 1024)).toBe('512 MB');
    expect(humanBytes(3 * 1024 ** 3)).toBe('3.0 GB');
  });

  it('never prints 10.0 or 1024 of a unit', () => {
    expect(humanBytes(Math.round(9.97 * 1024))).toBe('10 KB');
    expect(humanBytes(1024 * 1024 - 1)).toBe('1.0 MB');
  });

  it('renders invalid input as a dash', () => {
    expect(humanBytes(-1)).toBe('—');
    expect(humanBytes(Number.NaN)).toBe('—');
  });
});

describe('timeShort', () => {
  it('formats an ISO timestamp as local HH:MM:SS', () => {
    const d = new Date(2026, 9, 31, 7, 5, 9);
    expect(timeShort(d.toISOString())).toBe('07:05:09');
  });

  it('accepts epoch milliseconds', () => {
    const d = new Date(2026, 9, 31, 23, 59, 0);
    expect(timeShort(d.getTime())).toBe('23:59:00');
  });

  it('returns an empty string for an unparseable value', () => {
    expect(timeShort('not a date')).toBe('');
  });
});
