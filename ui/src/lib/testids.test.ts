import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

// Snapshot of the data-testid strings the e2e specs rely on, taken from the
// pre-redesign sources (815a1ab). A restyle must never drop one.
const PRE_CHANGE: Record<string, string[]> = {
  'routes/Intercept.svelte': [
    'drop', 'forward-all', 'forward', 'held-load-error', 'intercept-detail', 'intercept-error',
    'intercept-state', 'intercept-view', 'not-editable-badge', 'to-repeater',
  ],
  'routes/Scope.svelte': ['scope-view', 'strict-toggle'],
  'lib/components/HeldList.svelte': ['held-empty', 'held-list', 'held-row'],
  'lib/components/RulesEditor.svelte': [
    'rules-editor', 'rules-overflow', 'rules-save', 'rules-saved', 'rules-skip-ext',
    'rules-skip-oos', 'rules-timeout',
  ],
  'lib/components/ScopeRules.svelte': [
    'rule-also-apex', 'rule-down', 'rule-host-type', 'rule-host-value', 'rule-path-type',
    'rule-path-value', 'rule-ports', 'rule-remove', 'rule-row', 'rule-scheme', 'rule-up',
    'scope-error', 'scope-local-error', 'scope-rules', 'scope-save',
    // templated: data-testid={`add-${l.key}`} and data-testid={`scope-${l.key}`}
    'add-${l.key}', 'scope-${l.key}',
  ],
  'lib/components/ScopeImport.svelte': [
    'import-apply', 'import-discard', 'import-preview', 'import-run', 'import-text',
    'preview-rule', 'scope-import',
  ],
};

describe('data-testid preservation', () => {
  for (const [file, ids] of Object.entries(PRE_CHANGE)) {
    it(`${file} keeps its ${ids.length} testids`, () => {
      const src = readFileSync(resolve(process.cwd(), 'src', file), 'utf8');
      for (const id of ids) {
        expect(src, `${file} lost testid ${id}`).toMatch(
          new RegExp(`(?:data-)?testid=(?:"|\\{\`)${id.replace(/[$.{}]/g, '\\$&')}`),
        );
      }
    });
  }
});
