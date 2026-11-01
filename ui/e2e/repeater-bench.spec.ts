import { test, expect, type Request } from '@playwright/test';
import { b64, exId, mockBench, openRepeater, type Bench } from './fixtures/repeater-mock';

// U2 D2: the repeater bench — the new-tab template, send history with status
// badges, restore, compare (diff), the diff byte cap, the single error toast
// and draft persistence (title, history, collapsed strip). No backend: drafts
// live in an in-test store so a reload reads back what the page saved.

const lines = (n: number, change: (i: number) => string | null) =>
  Array.from({ length: n }, (_, i) => change(i) ?? `line ${i} same`).join('\n') + '\n';

const sentRaw = (r: Request) => Buffer.from((r.postDataJSON() as { raw: string }).raw, 'base64');

async function lastDraftRaw(bench: Bench, id?: string): Promise<string | undefined> {
  const p = [...bench.puts].reverse().find((x) => id === undefined || x.id === id);
  return p?.json.raw as string | undefined;
}

test('template tab: Host typed in the editor is the target; two sends, statuses, compare, restore', async ({ page }) => {
  const older = lines(60, (i) => (i === 30 ? 'value: old' : null));
  const newer = lines(60, (i) => (i === 30 ? 'value: new' : i === 59 ? 'tail added' : null));
  const bench = await mockBench(page, [
    { status: 200, body: Buffer.from(older) },
    { status: 404, body: Buffer.from(newer) },
  ]);
  await openRepeater(page);

  // New: the GET / template, caret on the Host header.
  await page.getByTestId('new-tab').click();
  const editor = page.getByTestId('repeater-request').locator('.cm-content');
  await expect(editor).toContainText('GET / HTTP/1.1');
  await expect(editor).toBeFocused();
  await expect(page.getByTestId('send-history-empty')).toBeVisible();
  await expect(page.getByTestId('compare-previous')).toBeDisabled();

  // Type only the Host header value, then Send.
  await page.keyboard.type('api.example.test');
  await page.getByTestId('send').click();
  await expect.poll(() => bench.sends.length).toBe(1);
  const first = bench.sends[0].postDataJSON() as { host: string; port: number; raw: string };
  expect(first.host).toBe('api.example.test');
  expect(first.port).toBe(80);
  expect(sentRaw(bench.sends[0]).toString('latin1')).toBe('GET / HTTP/1.1\r\nHost: api.example.test\r\n\r\n');
  await expect(page.getByTestId('conn-settings').locator('.head-target')).toHaveText('api.example.test:80');
  await expect(page.getByTestId('conn-host')).toHaveValue('api.example.test');

  // One send with a response: compare stays disabled.
  const rows = page.getByTestId('send-history-row');
  await expect(rows).toHaveCount(1);
  await expect(rows.nth(0).getByTestId('send-history-status')).toContainText('200');
  await expect(page.getByTestId('compare-previous')).toBeDisabled();

  // Second send: two rows, newest first, both with their status badge.
  await editor.click();
  await page.keyboard.press('ControlOrMeta+End');
  await page.keyboard.type('x');
  await page.getByTestId('send').click();
  await expect.poll(() => bench.sends.length).toBe(2);
  await expect(rows).toHaveCount(2);
  await expect(rows.nth(0).getByTestId('send-history-status')).toContainText('404');
  await expect(rows.nth(1).getByTestId('send-history-status')).toContainText('200');
  expect(bench.exchangeGets).toEqual(expect.arrayContaining([exId(0), exId(1)]));
  await expect(page.getByTestId('send-history-count')).toHaveText('2');

  // Compare with previous: changed lines, folds, hide-unchanged, close.
  const compare = page.getByTestId('compare-previous');
  await expect(compare).toBeEnabled();
  await compare.click();
  const diff = page.getByTestId('diff-view');
  await expect(diff).toBeVisible();
  // Head and body are compared: the status line, the changed value, the replaced last line.
  const dels = diff.locator('[data-testid="diff-line"][data-kind="del"]');
  const adds = diff.locator('[data-testid="diff-line"][data-kind="add"]');
  await expect(dels).toHaveCount(3);
  await expect(adds).toHaveCount(3);
  await expect(dels.filter({ hasText: 'HTTP/1.1 200 X' })).toHaveCount(1);
  await expect(dels.filter({ hasText: 'value: old' })).toHaveCount(1);
  await expect(dels.filter({ hasText: 'line 59 same' })).toHaveCount(1);
  await expect(adds.filter({ hasText: 'HTTP/1.1 404 X' })).toHaveCount(1);
  await expect(adds.filter({ hasText: 'value: new' })).toHaveCount(1);
  await expect(adds.filter({ hasText: 'tail added' })).toHaveCount(1);
  await expect(diff.getByTestId('diff-summary')).toHaveText(/\+3\s*-3/);
  const folds = diff.getByTestId('diff-fold');
  const foldCount = await folds.count();
  expect(foldCount).toBeGreaterThan(0);
  const shownBefore = await diff.getByTestId('diff-line').count();
  await folds.first().click();
  await expect(diff.getByTestId('diff-fold')).toHaveCount(foldCount - 1);
  expect(await diff.getByTestId('diff-line').count()).toBeGreaterThan(shownBefore);
  await diff.getByTestId('diff-hide-unchanged').uncheck();
  await expect(diff.getByTestId('diff-fold')).toHaveCount(0);
  await expect(diff.locator('[data-testid="diff-line"][data-kind="same"]').first()).toBeVisible();
  await diff.getByTestId('diff-close').click();
  await expect(diff).toHaveCount(0);
  await expect(page.getByTestId('result-resp-body')).toBeVisible();

  // Restore the first send over an edited draft: exact bytes, dirty, no send.
  await editor.click();
  await page.keyboard.press('ControlOrMeta+End');
  await page.keyboard.type('EDITED');
  const putsBefore = bench.puts.length;
  await rows.nth(1).click();
  await expect(page.getByTestId('send-history-dirty')).toBeVisible();
  await expect(page.getByTestId('send-history-notice')).toContainText('Not sent yet');
  const want = sentRaw(bench.sends[0]).toString('latin1');
  await expect.poll(async () => (bench.puts.length > putsBefore ? lastDraftRaw(bench) : undefined)).toBe(want);
  await page.waitForTimeout(300);
  expect(bench.sends.length).toBe(2);
  await expect(editor).toContainText('Host: api.example.test');
  await expect(editor).not.toContainText('EDITED');
  await expect(rows.nth(1)).toHaveAttribute('aria-current', 'true');
});

test('a draft with title and history loads; a truncated record restores its response only; the strip stays collapsed', async ({ page }) => {
  const tabId = '01HZZZZZZZZZZZZZZZZZZZTAB1';
  const raw = 'GET /draft HTTP/1.1\r\nHost: saved.test\r\n\r\n';
  const conn = { host: 'saved.test', port: 80, tls: false, reuse: false, follow: 'off' };
  const sha = 'ab'.repeat(32);
  const json = {
    raw, conn, mode: 'raw', title: 'Login probe',
    history: [
      { at: '2026-11-01T10:00:02Z', request_sha256: sha, request_len: 300_000, truncated: true, exchange_id: exId(7), status: 201, conn, mode: 'raw' },
      { at: '2026-11-01T10:00:01Z', request_sha256: 'cd'.repeat(32), request_len: raw.length, request: b64(raw), exchange_id: exId(8), status: 200, conn, mode: 'raw' },
    ],
  };
  const bench = await mockBench(page, [], [{ tab_id: tabId, json }], {
    [exId(7)]: { status: 201, body: Buffer.from('created') },
    [exId(8)]: { status: 200, body: Buffer.from('ok') },
  });
  await openRepeater(page);
  await expect(page.getByTestId('repeater-tab')).toHaveCount(1);
  await expect(page.getByTestId('repeater-tab-select')).toContainText('Login probe');
  const rows = page.getByTestId('send-history-row');
  await expect(rows).toHaveCount(2);
  await expect(rows.nth(0)).toContainText('resp only');

  // Restore the truncated record: notice, response shown, editor unchanged.
  const editor = page.getByTestId('repeater-request').locator('.cm-content');
  const before = await editor.innerText();
  await rows.nth(0).click();
  await expect(page.getByTestId('send-history-notice')).toContainText('too large to keep');
  await expect(page.getByTestId('send-history-notice')).toContainText('300000 bytes');
  await expect(page.getByTestId('result-resp-body').locator('.cm-content')).toContainText('created');
  expect(await editor.innerText()).toBe(before);
  await expect(page.getByTestId('send-history-dirty')).toHaveCount(0);
  expect(bench.sends).toHaveLength(0);

  // Collapse the strip; it and the draft survive a reload.
  await page.getByTestId('send-history-toggle').click();
  await expect(rows).toHaveCount(0);
  await page.reload();
  await page.getByTestId('nav-repeater').click();
  await expect(page.getByTestId('repeater-tab-select')).toContainText('Login probe');
  await expect(page.getByTestId('send-history-toggle')).toHaveAttribute('aria-expanded', 'false');
  await expect(page.getByTestId('send-history-count')).toHaveText('2');
  await expect(rows).toHaveCount(0);
  await page.getByTestId('send-history-toggle').click();
  await expect(rows).toHaveCount(2);
});

test('a rename and the send history persist through the draft across a reload', async ({ page }) => {
  const bench = await mockBench(page, [{ status: 200, body: Buffer.from('one') }]);
  await openRepeater(page);
  await page.getByTestId('conn-host').fill('persist.test');
  await page.getByTestId('send').click();
  await expect(page.getByTestId('send-history-row')).toHaveCount(1);
  await page.getByTestId('repeater-tab-select').dblclick();
  await page.getByTestId('repeater-tab-rename').fill('My bench');
  await page.getByTestId('repeater-tab-rename').press('Enter');
  await expect.poll(() => bench.puts.some((p) => p.json.title === 'My bench' && Array.isArray(p.json.history) && p.json.history.length === 1)).toBe(true);
  await page.reload();
  await page.getByTestId('nav-repeater').click();
  await expect(page.getByTestId('repeater-tab-select')).toContainText('My bench');
  await expect(page.getByTestId('send-history-row')).toHaveCount(1);
});

test('compare refuses a response over 2 MiB with diff-too-large', async ({ page }) => {
  const big = Buffer.alloc(2 * 1024 * 1024 + 16, 'a');
  const bench = await mockBench(page, [
    { status: 200, body: Buffer.from('small') },
    { status: 200, body: big },
  ]);
  await openRepeater(page);
  await page.getByTestId('conn-host').fill('big.test');
  await page.getByTestId('send').click();
  await expect(page.getByTestId('send-history-row')).toHaveCount(1);
  await page.getByTestId('send').click();
  await expect.poll(() => bench.sends.length).toBe(2);
  await expect(page.getByTestId('send-history-row')).toHaveCount(2);
  await page.getByTestId('compare-previous').click();
  await expect(page.getByTestId('diff-too-large')).toBeVisible();
  await expect(page.getByTestId('diff-line')).toHaveCount(0);
});

test('a 502 send shows exactly one error toast and a failed history row', async ({ page }) => {
  await mockBench(page, [{ error: 502 }]);
  await openRepeater(page);
  await page.getByTestId('conn-host').fill('down.test');
  await page.getByTestId('send').click();
  const toasts = page.locator('[data-testid="toast"][data-kind="error"]');
  await expect(toasts.first()).toContainText('Send failed');
  await page.waitForTimeout(500);
  await expect(toasts).toHaveCount(1);
  await expect(page.getByTestId('send-history-row')).toHaveCount(1);
  await expect(page.getByTestId('send-history-error')).toHaveText('failed');
});
