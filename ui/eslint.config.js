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
const SETATTR_SINK_MESSAGE =
  '§11.1: setAttribute() with srcdoc or an on* handler installs live markup / an executable handler from a string. Set text/data attributes only.';
const SPREAD_SINK_MESSAGE =
  '§11.1: a spread attribute carrying srcdoc or an on* handler hides a DOM/exec sink. Bind these explicitly, never through a spread of captured data.';
const DYNAMIC_URL_MESSAGE =
  '§11.1: a dynamic href/src/action bound to a non-literal could carry an attacker-controlled URL (e.g. javascript:). Allowlist the scheme/origin before binding.';

const restrictedProperties = [
  { property: 'innerHTML', message: HTML_SINK_MESSAGE },
  { property: 'outerHTML', message: HTML_SINK_MESSAGE },
  { property: 'insertAdjacentHTML', message: HTML_SINK_MESSAGE },
  { property: 'srcdoc', message: HTML_SINK_MESSAGE },
];

// Plain-JS/TS AST selectors (component <script> blocks and *.ts modules).
//   el.setAttribute('srcdoc'|'on*', …) -> installs markup / an executable handler
const jsRestrictedSyntax = [
  {
    // Matches a string-literal first arg of srcdoc or any on* event handler.
    selector:
      "CallExpression[callee.property.name='setAttribute'][arguments.0.value=/^(srcdoc|on\\w+)$/i]",
    message: SETATTR_SINK_MESSAGE,
  },
];

// Selectors evaluated against the svelte-eslint-parser AST (templates).
const svelteRestrictedSyntax = [
  ...jsRestrictedSyntax,
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
  {
    // Spread attribute <el {...x}> whose object literal carries a srcdoc/on*
    // KEY (identifier form, e.g. {...{ srcdoc: s }} / {...{ onclick: f }}).
    selector: "SvelteSpreadAttribute Property[key.name=/^(srcdoc|on\\w+)$/i]",
    message: SPREAD_SINK_MESSAGE,
  },
  {
    // Same, string-literal key form ({...{ 'onerror': f }}).
    selector: "SvelteSpreadAttribute Property[key.value=/^(srcdoc|on\\w+)$/i]",
    message: SPREAD_SINK_MESSAGE,
  },
  {
    // Dynamic href/src/action/… bound to a NON-literal expression. A literal
    // (string/template) is handled by the javascript: rule above; anything else
    // (identifier, member, call, ternary …) is an unvetted, possibly
    // attacker-controlled URL.
    selector:
      "SvelteAttribute[key.name=/^(href|xlink:href|src|formaction|action|data|poster)$/i] > SvelteMustacheTag[expression.type!='Literal'][expression.type!='TemplateLiteral']",
    message: DYNAMIC_URL_MESSAGE,
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
      'no-restricted-syntax': ['error', ...jsRestrictedSyntax],
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
      'no-restricted-syntax': ['error', ...jsRestrictedSyntax],
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
