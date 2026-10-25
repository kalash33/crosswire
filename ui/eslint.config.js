import js from '@eslint/js';
import tseslint from 'typescript-eslint';
import svelte from 'eslint-plugin-svelte';
import svelteParser from 'svelte-eslint-parser';
import globals from 'globals';

// ---------------------------------------------------------------------------
// §11.1 untrusted-content bans.
//
// Captured HTTP bytes (request/response bodies, headers, URLs) are rendered by
// this UI and must ONLY ever appear as text. Every API below can turn a string
// into live DOM or an executable URL, so each is a hard error over ui/src/.
//
//   {@html ...}             -> svelte/no-at-html-tags
//   el.innerHTML / outerHTML -> no-restricted-properties (member access)
//   el.insertAdjacentHTML    -> no-restricted-properties
//   iframe srcdoc            -> no-restricted-properties (JS) + svelte selector
//   javascript: href/src     -> no-script-url (JS strings) + svelte selector
//   <svelte:element this>    -> svelte no-restricted-syntax selector
// ---------------------------------------------------------------------------
const HTML_SINK_MESSAGE =
  '§11.1: this API renders a string as live DOM/markup. Captured bytes must render as text only (use textContent / DOM nodes).';
const SCRIPT_URL_MESSAGE =
  '§11.1: javascript: URLs execute. Captured bytes must never become an executable URL.';
const SVELTE_ELEMENT_MESSAGE =
  '§11.1: <svelte:element this={...}> can render an attacker-chosen tag. Use a fixed element.';

const restrictedProperties = [
  { property: 'innerHTML', message: HTML_SINK_MESSAGE },
  { property: 'outerHTML', message: HTML_SINK_MESSAGE },
  { property: 'insertAdjacentHTML', message: HTML_SINK_MESSAGE },
  { property: 'srcdoc', message: HTML_SINK_MESSAGE },
];

// Selectors evaluated against the svelte-eslint-parser AST (templates).
const svelteRestrictedSyntax = [
  {
    selector: "SvelteAttribute[key.name='srcdoc']",
    message: HTML_SINK_MESSAGE,
  },
  {
    selector:
      "SvelteAttribute[key.name=/^(href|xlink:href|src|formaction|action|data|poster)$/i] SvelteLiteral[value=/^\\s*javascript:/i]",
    message: SCRIPT_URL_MESSAGE,
  },
  {
    // <svelte:element> is parsed as a SvelteElement whose name is the literal
    // string "svelte:element".
    selector: "SvelteElement[name.name='svelte:element']",
    message: SVELTE_ELEMENT_MESSAGE,
  },
];

export default tseslint.config(
  {
    ignores: ['dist/', 'node_modules/', 'playwright-report/', 'test-results/'],
  },

  js.configs.recommended,
  ...tseslint.configs.recommended,
  ...svelte.configs.recommended,

  // TypeScript / JS source under ui/src.
  {
    files: ['src/**/*.{ts,js}'],
    languageOptions: {
      globals: { ...globals.browser },
    },
    rules: {
      'no-script-url': 'error',
      'no-restricted-properties': ['error', ...restrictedProperties],
    },
  },

  // Rune modules (*.svelte.ts): svelte-eslint-parser needs the TS parser for
  // type syntax. Same §11.1 sink bans as plain TS.
  {
    files: ['src/**/*.svelte.ts'],
    languageOptions: {
      parser: svelteParser,
      parserOptions: { parser: tseslint.parser },
      globals: { ...globals.browser },
    },
    rules: {
      'no-script-url': 'error',
      'no-restricted-properties': ['error', ...restrictedProperties],
    },
  },

  // Svelte components under ui/src.
  {
    files: ['src/**/*.svelte'],
    languageOptions: {
      parser: svelteParser,
      parserOptions: {
        parser: tseslint.parser,
      },
      globals: { ...globals.browser },
    },
    rules: {
      // {@html ...}
      'svelte/no-at-html-tags': 'error',
      // sinks usable from the component <script> block
      'no-script-url': 'error',
      'no-restricted-properties': ['error', ...restrictedProperties],
      // template-level sinks
      'no-restricted-syntax': ['error', ...svelteRestrictedSyntax],
    },
  },

  // Config / test files run in Node and may use Node globals.
  {
    files: ['*.config.{ts,js}', 'src/**/*.test.ts', 'e2e/**/*.ts'],
    languageOptions: {
      globals: { ...globals.node },
    },
  },
);
