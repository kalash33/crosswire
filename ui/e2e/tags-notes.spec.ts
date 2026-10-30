import { test, expect, type Page } from '@playwright/test';

// vite-preview + a stateful mocked API. PATCH echoes the merged exchange.

const A = '01HZZZZZZZZZZZZZZZZZZZZZAAA';
const B = '01HZZZZZZZZZZZZZZZZZZZZZBBB';
const b64 = (s: string) => Buffer.from(s, 'utf8').toString('base64');
const HEAD = 'GET / HTTP/1.1\r\nHost: a.example\r\n\r\n';

interface Ex { id: string; tags: string[]; notes: string }

function exchangeJson(e: Ex) {
  return {
    id: e.id,
    request: {
      kind: 'http1-raw', fidelity: 'raw', diagnostics: [],
      head: { type: 'http1-raw', bytes: b64(HEAD), parser_version: 1 },
      body: { type: 'none' },
    },
    response: null, scope_state: 'in', state: 'committed', tags: e.tags, notes: e.notes,
  };
}

interface Opts {
  patchStatus?: number;
  /** Resolves before a PATCH for this id is answered. */
  gate?: (id: string, body: { tags?: string[]; notes?: string }) => Promise<void> | undefined;
  /** Per-PATCH status override (default 200). */
  statusFor?: (body: { tags?: string[]; notes?: string }) => number;
}

async function setup(page: Page, opts: Opts = {}) {
  const db: Record<string, Ex> = {
    [A]: { id: A, tags: ['old'], notes: '' },
    [B]: { id: B, tags: ['other'], notes: 'bee' },
  };
  const patches: { id: string; body: Record<string, unknown> }[] = [];
  await page.route('**/api/v1/counts', (r) => r.fulfill({ status: 200, json: { exchanges: 2 } }));
  await page.route(/\/exchanges\/[^/?]+\/body\//, (r) =>
    r.fulfill({ status: 404, json: { error: { code: 'not_found', message: 'x' } } }),
  );
  await page.route(/\/api\/v1\/exchanges(\?.*)?$/, (r) => {
    const tag = new URL(r.request().url()).searchParams.get('tag');
    const items = Object.values(db)
      .filter((e) => !tag || e.tags.includes(tag))
      .map((e) => ({
        id: e.id, created_at: 1_700_000_000_000, method: 'GET', host: 'a.example',
        path: `/${e.id.slice(-3)}`, status: 200, resp_len: 1, state: 'committed', tags: e.tags,
      }));
    return r.fulfill({ status: 200, json: { items, next_cursor: null } });
  });
  await page.route(/\/api\/v1\/exchanges\/[^/?]+$/, async (r) => {
    const id = decodeURIComponent(new URL(r.request().url()).pathname.split('/').pop()!);
    if (r.request().method() === 'PATCH') {
      const body = r.request().postDataJSON() as { tags?: string[]; notes?: string };
      patches.push({ id, body });
      await opts.gate?.(id, body);
      const status = opts.statusFor?.(body) ?? opts.patchStatus ?? 200;
      if (status >= 400) {
        return r.fulfill({ status, json: { error: { code: 'internal', message: 'boom' } } });
      }
      if (body.tags) db[id].tags = body.tags;
      if (body.notes !== undefined) db[id].notes = body.notes;
      // Like the real route: 200 with an EMPTY body.
      return r.fulfill({ status: 200, body: '' });
    }
    return r.fulfill({ status: 200, json: exchangeJson(db[id]) });
  });
  await page.goto('/');
  return { patches };
}

const rowFor = (page: Page, id: string) => page.locator(`[data-testid="history-row"][data-id="${id}"]`);

test('add_tag_patches_full_list_and_updates_row_and_detail', async ({ page }) => {
  const { patches } = await setup(page);
  await rowFor(page, A).locator('td.open').first().click();
  await expect(page.getByTestId('tag-chip')).toHaveCount(1);
  await page.getByTestId('tag-input').fill('new');
  await page.getByTestId('tag-input').press('Enter');
  await expect(page.getByTestId('tag-chip')).toHaveCount(2);
  await expect(rowFor(page, A).locator('td.tags')).toContainText('new');
  expect(patches).toEqual([{ id: A, body: { tags: ['old', 'new'] } }]);
});

test('remove_tag_patches_and_removes_chip', async ({ page }) => {
  const { patches } = await setup(page);
  await rowFor(page, A).locator('td.open').first().click();
  await page.getByTestId('tag-remove').first().click();
  await expect(page.getByTestId('tag-chip')).toHaveCount(0);
  await expect(rowFor(page, A).locator('td.tags')).not.toContainText('old');
  expect(patches).toEqual([{ id: A, body: { tags: [] } }]);
});

test('notes_typing_sends_one_debounced_patch', async ({ page }) => {
  const { patches } = await setup(page);
  await rowFor(page, A).locator('td.open').first().click();
  await page.getByTestId('notes-input').click();
  await page.getByTestId('notes-input').pressSequentially('abcdefgh', { delay: 10 });
  await page.waitForTimeout(600);
  expect(patches).toEqual([{ id: A, body: { notes: 'abcdefgh' } }]);
});

test('patch_500_rolls_back_and_shows_error', async ({ page }) => {
  await setup(page, { patchStatus: 500 });
  await rowFor(page, A).locator('td.open').first().click();
  await page.getByTestId('tag-input').fill('new');
  await page.getByTestId('tag-input').press('Enter');
  await expect(page.getByTestId('tags-notes-error')).toBeVisible();
  await expect(page.getByTestId('tag-chip')).toHaveCount(1);
  await expect(page.getByTestId('tag-chip')).toContainText('old');
});

test('switching_exchange_mid_flight_discards_response', async ({ page }) => {
  let release!: () => void;
  const held = new Promise<void>((res) => (release = res));
  await setup(page, { gate: (id) => (id === A ? held : undefined) });
  await rowFor(page, A).locator('td.open').first().click();
  await page.getByTestId('tag-input').fill('late');
  await page.getByTestId('tag-input').press('Enter');
  await rowFor(page, B).locator('td.open').first().click();
  await expect(page.getByTestId('detail-id')).toHaveText(B);
  release();
  await page.waitForTimeout(300);
  await expect(page.getByTestId('detail-id')).toHaveText(B);
  await expect(page.getByTestId('tag-chip')).toHaveCount(1);
  await expect(page.getByTestId('tag-chip')).toContainText('other');
  await expect(page.getByTestId('notes-input')).toHaveValue('bee');
});

test('row_tag_action_focuses_tag_input', async ({ page }) => {
  await setup(page);
  await rowFor(page, A).focus();
  await page.getByTestId('row-tag').click();
  await expect(page.getByTestId('detail-id')).toHaveText(A);
  await expect(page.getByTestId('tag-input')).toBeFocused();
});

test('filter_tag_lists_tagged_row', async ({ page }) => {
  await setup(page);
  await page.getByTestId('filter-tag').fill('other');
  await page.getByTestId('filter-apply').click();
  await expect(page.getByTestId('history-row')).toHaveCount(1);
  await expect(rowFor(page, B)).toBeVisible();
});

test('stale_tag_failure_does_not_roll_back_newer_state', async ({ page }) => {
  let releaseX!: () => void;
  const heldX = new Promise<void>((res) => (releaseX = res));
  const { patches } = await setup(page, {
    gate: (_id, body) => (body.tags?.includes('x') && !body.tags.includes('y') ? heldX : undefined),
    statusFor: (body) => (body.tags?.includes('x') && !body.tags.includes('y') ? 500 : 200),
  });
  await rowFor(page, A).locator('td.open').first().click();
  await page.getByTestId('tag-input').fill('x');
  await page.getByTestId('tag-input').press('Enter');
  await page.getByTestId('tag-input').fill('y');
  await page.getByTestId('tag-input').press('Enter');
  await expect(page.getByTestId('tag-chip')).toHaveCount(3);
  await expect.poll(() => patches.length).toBe(2);
  releaseX();
  await page.waitForTimeout(300);
  await expect(page.getByTestId('tag-chip')).toHaveCount(3);
  await expect(page.getByTestId('tags-notes-error')).toHaveCount(0);
  await expect(rowFor(page, A).locator('td.tags')).toContainText('y');
});

test('notes_failure_keeps_text_typed_since', async ({ page }) => {
  let release!: () => void;
  const held = new Promise<void>((res) => (release = res));
  const { patches } = await setup(page, {
    gate: (_id, body) => (body.notes === 'abc' ? held : undefined),
    statusFor: (body) => (body.notes === 'abc' ? 500 : 200),
  });
  await rowFor(page, A).locator('td.open').first().click();
  await page.getByTestId('notes-input').click();
  await page.getByTestId('notes-input').pressSequentially('abc');
  await expect.poll(() => patches.length).toBe(1);
  await page.getByTestId('notes-input').pressSequentially('def');
  release();
  await expect(page.getByTestId('tags-notes-error')).toBeVisible();
  await expect(page.getByTestId('notes-input')).toHaveValue('abcdef');
});
