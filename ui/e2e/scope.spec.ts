import { test, expect, type Page } from '@playwright/test';

// No backend: `vite preview` serves the bundle; every API call is mocked.

const COUNTS = {
  bytes_inline: 0, bytes_packs: 0, committed: 0, error: 0, exchanges: 0,
  history: 0, intruder: 0, markers: 0, repeater: 0,
};
const RULES = {
  skip_extensions: [], skip_out_of_scope: false, overflow: 'forward-newest', on_timeout: 'forward',
};
const LIVE = {
  include: [{ host: { type: 'exact', value: 'a.test' }, ports: [443], scheme: 'https' }],
  exclude: [{ host: { type: 'suffix', value: { suffix: 'x.test', also_apex: true } } }],
};
const IMPORTED = {
  include: [
    { host: { type: 'exact', value: 'one.test' } },
    { host: { type: 'suffix', value: { suffix: 'two.test', also_apex: false } } },
  ],
  exclude: [],
};

interface Harness {
  puts: unknown[];
  imports: unknown[];
  strict: unknown[];
}

async function mockApi(page: Page, opts: { putStatus?: number } = {}): Promise<Harness> {
  const h: Harness = { puts: [], imports: [], strict: [] };
  let live: unknown = LIVE;
  await page.route('**/api/v1/counts', (r) => r.fulfill({ status: 200, json: COUNTS }));
  await page.route(/\/api\/v1\/repeater\/tabs/, (r) => r.fulfill({ status: 200, json: [] }));
  await page.route(/\/api\/v1\/intercept$/, (r) =>
    r.fulfill({ status: 200, json: { enabled: false, rules: RULES, held: [] } }),
  );
  await page.route(/\/api\/v1\/scope$/, (r) => {
    if (r.request().method() === 'PUT') {
      h.puts.push(r.request().postDataJSON());
      if (opts.putStatus === 422) {
        return r.fulfill({
          status: 422,
          json: { error: { code: 'scope_invalid', message: 'scope did not compile: invalid regex: (' } },
        });
      }
      live = r.request().postDataJSON();
      return r.fulfill({ status: 200, body: '' });
    }
    return r.fulfill({ status: 200, json: live });
  });
  await page.route(/\/api\/v1\/scope\/import$/, (r) => {
    h.imports.push(r.request().postDataJSON());
    r.fulfill({ status: 200, json: IMPORTED });
  });
  await page.route(/\/api\/v1\/settings\/enforce_scope_strictly$/, (r) => {
    h.strict.push(r.request().postDataJSON());
    r.fulfill({ status: 200, body: '' });
  });
  return h;
}

async function openScope(page: Page) {
  await page.goto('/');
  await expect(page.getByTestId('app-shell')).toBeVisible();
  await page.getByTestId('nav-scope').click();
  await expect(page.getByTestId('scope-view')).toBeVisible();
  await expect(page.getByTestId('rule-row')).toHaveCount(2);
}

test('editing a rule and Save round-trips the PUT', async ({ page }) => {
  const h = await mockApi(page);
  await openScope(page);
  const first = page.getByTestId('rule-row').first();
  await expect(first.getByTestId('rule-host-value')).toHaveValue('a.test');
  await first.getByTestId('rule-host-value').fill('b.test');
  await first.getByTestId('rule-ports').fill('443, 8443');
  await page.getByTestId('add-include').click();
  await page.getByTestId('rule-row').nth(1).getByTestId('rule-host-value').fill('c.test');
  await page.getByTestId('scope-save').click();
  await expect.poll(() => h.puts.length).toBe(1);
  expect(h.puts[0]).toEqual({
    include: [
      { host: { type: 'exact', value: 'b.test' }, ports: [443, 8443], scheme: 'https' },
      { host: { type: 'exact', value: 'c.test' } },
    ],
    exclude: LIVE.exclude,
  });
});

test('pasting a program list previews parsed rules, then apply PUTs them', async ({ page }) => {
  const h = await mockApi(page);
  await openScope(page);
  await page.getByTestId('import-text').fill('one.test\n*.two.test');
  await page.getByTestId('import-run').click();
  expect(h.imports).toEqual([{ text: 'one.test\n*.two.test' }]);
  await expect(page.getByTestId('preview-rule')).toHaveCount(2);
  await expect(page.getByTestId('import-preview')).toContainText('one.test');
  expect(h.puts).toEqual([]); // preview is not saved
  await page.getByTestId('import-apply').click();
  await expect.poll(() => h.puts).toEqual([IMPORTED]);
  await expect(page.getByTestId('import-preview')).toHaveCount(0);
  await expect(page.getByTestId('rule-row')).toHaveCount(2);
  await expect(page.getByTestId('rule-row').first().getByTestId('rule-host-value')).toHaveValue('one.test');
});

test('a 422 shows the error and leaves the editor populated with the edits', async ({ page }) => {
  await mockApi(page, { putStatus: 422 });
  await openScope(page);
  const first = page.getByTestId('rule-row').first();
  await first.getByTestId('rule-host-type').selectOption('regex');
  await first.getByTestId('rule-host-value').fill('(');
  await page.getByTestId('scope-save').click();
  await expect(page.getByTestId('scope-error')).toContainText('invalid regex');
  await expect(page.getByTestId('rule-row')).toHaveCount(2);
  await expect(first.getByTestId('rule-host-value')).toHaveValue('(');
  // The live scope (status bar) is unchanged.
  await expect(page.getByTestId('status-scope')).toContainText('1 in / 1 out');
});

test('the enforce-strict toggle PUTs {enabled}', async ({ page }) => {
  const h = await mockApi(page);
  await openScope(page);
  await page.getByTestId('strict-toggle').click();
  await expect.poll(() => h.strict).toEqual([{ enabled: true }]);
  await expect(page.getByTestId('status-scope')).toContainText('strict');
});
