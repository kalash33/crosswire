// Display helpers shared by the views. Pure; no locale-dependent separators.

const UNITS = ['B', 'KB', 'MB', 'GB', 'TB'];

/**
 * Human-readable byte size, base 1024: `0 B`, `1.8 KB`, `15 KB`, `2.3 MB`.
 * One decimal below 10 of a unit, none above; never `10.0 KB` or `1024 KB`.
 */
export function humanBytes(n: number): string {
  if (!Number.isFinite(n) || n < 0) return '—';
  if (n < 1024) return `${Math.round(n)} B`;
  let v = n;
  let u = 0;
  while (v >= 1024 && u < UNITS.length - 1) {
    v /= 1024;
    u++;
  }
  let text = v < 10 ? v.toFixed(1) : String(Math.round(v));
  if (text === '10.0') text = '10';
  if (text === '1024' && u < UNITS.length - 1) {
    u++;
    text = '1.0';
  }
  return `${text} ${UNITS[u]}`;
}

const pad = (n: number) => String(n).padStart(2, '0');

/** Local wall-clock `HH:MM:SS` for an ISO string or epoch milliseconds; '' if invalid. */
export function timeShort(iso: string | number): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return '';
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}
