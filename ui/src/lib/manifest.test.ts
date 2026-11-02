import { describe, expect, it } from 'vitest';
import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';

// The installable-app manifest and icon set (ui/public/, copied to the bundle
// root by Vite). Their colours are literals (a manifest and an SVG are not
// CSS), so this pins them to the dark theme tokens in theme.css.

const UI = process.cwd();
const read = (p: string) => readFileSync(join(UI, p), 'utf8');

/** A token's value in the default (dark) `:root` block of theme.css. */
function darkToken(name: string): string {
  const css = read('src/lib/theme.css');
  const root = css.slice(css.indexOf(':root {'), css.indexOf('}', css.indexOf(':root {')));
  const m = new RegExp(`--${name}:\\s*([^;]+);`).exec(root);
  if (!m) throw new Error(`no --${name} in the dark :root`);
  return m[1].trim().toLowerCase();
}

interface Manifest {
  name: string;
  short_name: string;
  start_url: string;
  display: string;
  background_color: string;
  theme_color: string;
  icons: { src: string; sizes: string; type: string; purpose?: string }[];
}

describe('manifest.webmanifest', () => {
  const m = JSON.parse(read('public/manifest.webmanifest')) as Manifest;

  it('is a standalone app named Crosswire starting at /', () => {
    expect(m.name).toBe('Crosswire');
    expect(m.short_name).toBe('Crosswire');
    expect(m.display).toBe('standalone');
    expect(m.start_url).toBe('/');
  });

  it('uses the dark --bg token for its background and theme colours', () => {
    expect(m.background_color.toLowerCase()).toBe(darkToken('bg'));
    expect(m.theme_color.toLowerCase()).toBe(darkToken('bg'));
  });

  it('lists 192 and 512 PNGs and a maskable SVG, all present on disk', () => {
    const by = (sizes: string) => m.icons.find((i) => i.sizes === sizes);
    expect(by('192x192')).toMatchObject({ type: 'image/png' });
    expect(by('512x512')).toMatchObject({ type: 'image/png' });
    expect(by('any')).toMatchObject({ type: 'image/svg+xml', purpose: 'any maskable' });
    for (const icon of m.icons) {
      expect(icon.src.startsWith('/'), icon.src).toBe(true);
      expect(existsSync(join(UI, 'public', icon.src)), icon.src).toBe(true);
    }
  });

  it('PNG icons are real PNGs of the declared size', () => {
    for (const size of [192, 512]) {
      const png = readFileSync(join(UI, `public/icons/icon-${size}.png`));
      expect(png.subarray(1, 4).toString('latin1')).toBe('PNG');
      expect(png.readUInt32BE(16)).toBe(size); // IHDR width
      expect(png.readUInt32BE(20)).toBe(size); // IHDR height
    }
  });
});

describe('icon.svg', () => {
  const svg = read('public/icons/icon.svg');

  it('draws only with the dark --bg and --accent tokens', () => {
    const colours = new Set([...svg.matchAll(/(?:fill|stroke)="(#[0-9a-fA-F]+)"/g)].map((x) => x[1].toLowerCase()));
    expect(colours).toEqual(new Set([darkToken('bg'), darkToken('accent')]));
  });

  it('is static: no script, no external reference', () => {
    expect(svg).not.toMatch(/<script|href=|url\(|on[a-z]+=/i);
  });
});

describe('index.html', () => {
  const html = read('index.html');

  it('links the SVG favicon, the apple-touch-icon and the manifest', () => {
    expect(html).toContain('<link rel="icon" type="image/svg+xml" href="/icons/icon.svg" />');
    expect(html).toContain('<link rel="apple-touch-icon" href="/icons/icon-192.png" />');
    expect(html).toContain('<link rel="manifest" href="/manifest.webmanifest" />');
  });
});
