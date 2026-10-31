import { describe, expect, it } from 'vitest';
import { existsSync, readdirSync, readFileSync } from 'node:fs';
import { join, relative } from 'node:path';

// Tokens-only gate (U1 spec D1): colour literals live in ui/src/lib/theme.css
// and nowhere else; every component styles itself with var(--token).
//
// Heuristic (kept deliberately simple):
//  * Only CSS declaration VALUES are searched: `prop: value;` / `prop: value}`.
//    A declaration is a `name:` at the start of a block or after `;`, whose
//    value runs to the next `;` or `}`. A selector ends at `{`, so CSS id
//    selectors (`#app {`) and pseudo-classes never count.
//  * .css files: the whole file (comments stripped). .svelte files: only the
//    <style> blocks. .ts files: the whole file, which also catches style
//    objects such as `{ color: '#' + 'fff' }` written literally, plus any
//    string literal that is exactly a colour. Strings like `'#bootstrap='` never match:
//    the literal must be 3, 4, 6 or 8 hex digits and nothing else.
//  * A match is `#` + 3/4/6/8 hex digits on a word boundary.

// Vitest runs with the ui/ package as cwd (jsdom rewrites import.meta.url).
const SRC = join(process.cwd(), 'src');
const ALLOWED = new Set(['lib/theme.css']);
const HEX = /#(?:[0-9a-fA-F]{8}|[0-9a-fA-F]{6}|[0-9a-fA-F]{3,4})\b/g;
const DECL = /(?:^|[{;])\s*-{0,2}[a-zA-Z][-a-zA-Z0-9]*\s*:([^;{}]*)(?=[;}])/g;
const QUOTED = /(['"`])(#(?:[0-9a-fA-F]{8}|[0-9a-fA-F]{6}|[0-9a-fA-F]{3,4}))\1/g;

function stripComments(css: string): string {
  // Keep newlines so reported line numbers stay right.
  return css.replace(/\/\*[\s\S]*?\*\//g, (c) => c.replace(/[^\n]/g, ' '));
}

function lineOf(text: string, index: number): number {
  return text.slice(0, index).split('\n').length;
}

/** Colour literals in CSS declaration values of `css`, as [offset, literal]. */
function cssColours(css: string): [number, string][] {
  const out: [number, string][] = [];
  for (const m of css.matchAll(DECL)) {
    const value = m[1];
    const valueStart = (m.index ?? 0) + m[0].length - value.length;
    for (const h of value.matchAll(HEX)) out.push([valueStart + (h.index ?? 0), h[0]]);
  }
  return out;
}

/** Offenders in one source file, as `line: literal`. */
export function findColourLiterals(name: string, text: string): string[] {
  const hits: [number, string][] = [];
  if (name.endsWith('.css')) {
    hits.push(...cssColours(stripComments(text)));
  } else if (name.endsWith('.svelte')) {
    for (const m of text.matchAll(/<style[^>]*>([\s\S]*?)<\/style>/g)) {
      const start = (m.index ?? 0) + m[0].indexOf(m[1]);
      for (const [i, lit] of cssColours(stripComments(m[1]))) hits.push([start + i, lit]);
    }
  } else if (name.endsWith('.ts')) {
    hits.push(...cssColours(text));
    for (const m of text.matchAll(QUOTED)) hits.push([(m.index ?? 0) + 1, m[2]]);
  }
  const seen = new Set<string>();
  return hits
    .sort((a, b) => a[0] - b[0])
    .map(([i, lit]) => `${lineOf(text, i)}: ${lit}`)
    .filter((s) => !seen.has(s) && seen.add(s) !== undefined);
}

function walk(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((e) => {
    const p = join(dir, e.name);
    if (e.isDirectory()) return walk(p);
    return /\.(svelte|ts|css)$/.test(e.name) ? [p] : [];
  });
}

// Fixtures are assembled at runtime so this file holds no literal colour.
const H = '#';

describe('colour-literal heuristic', () => {
  it('flags colours in declaration values', () => {
    expect(findColourLiterals('a.css', `a { color: ${H}fff; }`)).toEqual([`1: ${H}fff`]);
    expect(findColourLiterals('a.css', `a{border:1px solid ${H}8884}`)).toEqual([`1: ${H}8884`]);
    expect(
      findColourLiterals('a.svelte', `<p>x</p>\n<style>\n  .x {\n    background: ${H}22333a;\n  }\n</style>`),
    ).toEqual([`4: ${H}22333a`]);
    expect(findColourLiterals('a.ts', `const t = { color: '${H}abcdef' };`)).toEqual([`1: ${H}abcdef`]);
    expect(findColourLiterals('a.ts', `const c = "${H}123";`)).toEqual([`1: ${H}123`]);
  });

  it('ignores id selectors, fragments and non-style markup', () => {
    expect(findColourLiterals('a.css', `${H}app { margin: 0; }\n${H}fff, ${H}add > b { color: red; }`)).toEqual([]);
    expect(findColourLiterals('a.ts', `if (location.hash.includes('${H}bootstrap=')) go('${H}cafe-route');`)).toEqual([]);
    expect(findColourLiterals('a.svelte', `<a data-x="${H}fff">${H}fff</a>`)).toEqual([]);
    expect(findColourLiterals('a.css', `/* was ${H}fff; */ a { color: var(--fg); }`)).toEqual([]);
  });
});

describe('no hex colours outside theme.css', () => {
  it('every stylesheet and component uses tokens', () => {
    expect(existsSync(join(SRC, 'main.ts')), `ui/src not found from ${process.cwd()}`).toBe(true);
    const offenders = walk(SRC).flatMap((file) => {
      const rel = relative(SRC, file).split('\\').join('/');
      if (ALLOWED.has(rel)) return [];
      return findColourLiterals(rel, readFileSync(file, 'utf8')).map((hit) => `${rel}:${hit}`);
    });
    expect(offenders, `colour literals outside theme.css:\n${offenders.join('\n')}`).toEqual([]);
  });
});
