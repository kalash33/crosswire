import { test, expect, type Page, type WebSocketRoute } from '@playwright/test';
import { mockApp, mockEvents, type Row } from './fixtures/app-mock';

// V1 D1: the virtualised History list (spec §3.1, §3.3.3). The DOM holds only
// the rows around the viewport plus two spacer rows; focus, selection, live
// prepends, paging and ARIA must behave as if every loaded row were there.
// Backend-free: the list and the events socket are mocked.

const N = 5_000;
const BASE = 1_700_000_000_000;

function gen(n: number, prefix = 'r', patch?: (i: number) => Partial<Row>): Row[] {
  const out: Row[] = [];
  for (let i = 0; i < n; i++) {
    out.push({
      id: `${prefix}${String(i).padStart(6, '0')}`,
      created_at: BASE - i * 1_000, // newest first, so the store order is the id order
      method: 'GET',
      host: `h${i % 7}.example`,
      path: `/p/${i}`,
      status: 200,
      resp_len: 10,
      state: 'committed',
      tags: [],
      ...(patch?.(i) ?? {}),
    });
  }
  return out;
}

interface Harness {
  afters: (string | null)[];
  pageErrors: string[];
  sockets: WebSocketRoute[];
}

/**
 * Serve `all` in keyset pages of `pageSize` (the mock ignores `limit`, so a
 * large `pageSize` makes a few big pages); `next_cursor` is the page's last id.
 */
async function serve(page: Page, all: Row[], pageSize: number): Promise<Harness> {
  const h: Harness = { afters: [], pageErrors: [], sockets: [] };
  page.on('pageerror', (e) => h.pageErrors.push(e.message));
  await mockApp(page);
  h.sockets = await mockEvents(page);
  await page.route(/\/api\/v1\/exchanges(\?.*)?$/, (r) => {
    const after = new URL(r.request().url()).searchParams.get('after');
    h.afters.push(after);
    const from = after === null ? 0 : all.findIndex((x) => x.id === after) + 1;
    const items = all.slice(from, from + pageSize);
    const more = from + pageSize < all.length;
    return r.fulfill({ status: 200, json: { items, next_cursor: more ? items[items.length - 1].id : null } });
  });
  return h;
}

const rows = (page: Page) => page.getByTestId('history-row');
const list = (page: Page) => page.getByTestId('history-list');

async function open(page: Page, firstPage: number) {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/');
  await expect(page.getByTestId('history-count')).toContainText(`${firstPage} loaded`);
  await expect(rows(page).first()).toBeVisible();
}

/** Set the list's scrollTop and wait two frames (scroll event + window pass). */
async function scrollListTo(page: Page, px: number): Promise<void> {
  await list(page).evaluate(async (el, y) => {
    el.scrollTop = y;
    await new Promise<void>((r) => requestAnimationFrame(() => requestAnimationFrame(() => r())));
  }, px);
}

const frames = (page: Page) =>
  page.evaluate(() => new Promise<void>((r) => requestAnimationFrame(() => requestAnimationFrame(() => r()))));

const rowH = (page: Page) =>
  rows(page).first().evaluate((el) => el.getBoundingClientRect().height);

const loadedCount = async (page: Page): Promise<number> => {
  const t = (await page.getByTestId('history-count').textContent()) ?? '';
  return Number(/(\d+) loaded/.exec(t)?.[1] ?? '0');
};

/** Scroll to the bottom until `total` rows are loaded (auto-load pages). */
async function loadAll(page: Page, total: number): Promise<void> {
  for (let i = 0; i < 1_000; i++) {
    const n = await loadedCount(page);
    if (n >= total) return;
    await scrollListTo(page, 1e9);
    await expect.poll(() => loadedCount(page)).toBeGreaterThan(n);
  }
}

async function expectBounded(page: Page): Promise<void> {
  const m = await list(page).evaluate((el) => {
    const r = el.querySelector('tr[data-testid="history-row"]');
    const lr = el.getBoundingClientRect();
    const head = el.querySelector('thead th')!.getBoundingClientRect();
    return {
      count: el.querySelectorAll('tr[data-testid="history-row"]').length,
      ch: el.clientHeight,
      rh: r ? r.getBoundingClientRect().height : 0,
      pads: el.querySelectorAll('tr.pad[data-testid="history-pad-top"], tr.pad[data-testid="history-pad-bottom"]').length,
      headOffset: head.top - lr.top,
    };
  });
  expect(m.rh).toBeGreaterThan(0);
  expect(m.count).toBeGreaterThan(0);
  expect(m.count).toBeLessThanOrEqual(2 * Math.ceil(m.ch / m.rh) + 8);
  expect(m.pads).toBe(2);
  expect(Math.abs(m.headOffset)).toBeLessThanOrEqual(1);
}

const focusedRowId = (page: Page) =>
  page.evaluate(
    () => (document.activeElement as HTMLElement | null)?.closest('tr[data-testid="history-row"]')?.getAttribute('data-id') ?? null,
  );

const activeTestId = (page: Page) =>
  page.evaluate(() => (document.activeElement as HTMLElement | null)?.getAttribute('data-testid') ?? document.activeElement?.tagName ?? null);

/** True when the row's box lies inside the list's box below the sticky header and above the footer. */
const rowInView = (page: Page, id: string) =>
  list(page).evaluate((el, rid) => {
    const r = el.querySelector(`tr[data-testid="history-row"][data-id="${rid}"]`);
    if (!r) return false;
    const b = r.getBoundingClientRect();
    const head = el.querySelector('thead th')!.getBoundingClientRect();
    const foot = el.querySelector('.foot')!.getBoundingClientRect();
    return b.top >= head.bottom - 1 && b.bottom <= foot.top + 1;
  }, id);

/** The first row fully below the sticky header, and its offset from the list top. */
const firstVisible = (page: Page) =>
  list(page).evaluate((el) => {
    const lr = el.getBoundingClientRect();
    const head = el.querySelector('thead th')!.getBoundingClientRect();
    for (const r of el.querySelectorAll<HTMLElement>('tr[data-testid="history-row"]')) {
      const b = r.getBoundingClientRect();
      if (b.top >= head.bottom - 0.5) return { id: r.dataset.id ?? '', offset: b.top - lr.top };
    }
    return null;
  });

test('the DOM stays bounded at the top, 25%, 50% and the bottom; pads and sticky header hold', async ({ page }) => {
  const all = gen(N);
  const h = await serve(page, all, N);
  await open(page, N);
  await expectBounded(page);
  const max = await list(page).evaluate((el) => el.scrollHeight - el.clientHeight);
  for (const f of [0.25, 0.5, 1]) {
    await scrollListTo(page, Math.round(max * f));
    await expectBounded(page);
  }
  // At the bottom the last loaded row is rendered and the bottom pad is empty.
  await expect(page.locator(`tr[data-testid="history-row"][data-id="${all[N - 1].id}"]`)).toHaveCount(1);
  expect(h.pageErrors).toEqual([]);
});

test('aria-rowcount is -1 while pages remain and n+1 at the end; aria-rowindex follows the # cell', async ({ page }) => {
  test.setTimeout(120_000);
  const all = gen(N);
  const h = await serve(page, all, 100);
  await open(page, 100);
  const table = list(page).locator('table');
  await expect(table).toHaveAttribute('aria-rowcount', '-1');
  await expect(list(page).locator('thead tr')).toHaveAttribute('aria-rowindex', '1');
  await loadAll(page, N);
  await expect(page.getByTestId('load-more')).toHaveText('End');
  await expect(table).toHaveAttribute('aria-rowcount', String(N + 1));
  const max = await list(page).evaluate((el) => el.scrollHeight - el.clientHeight);
  await scrollListTo(page, Math.round(max / 2));
  const first = rows(page).first();
  const idx = Number((await first.locator('td.idx').textContent())?.trim());
  expect(idx).toBeGreaterThan(1);
  await expect(first).toHaveAttribute('aria-rowindex', String(idx + 1));
  // Pads carry no row semantics.
  await expect(page.getByTestId('history-pad-top')).toHaveAttribute('aria-hidden', 'true');
  await expect(page.getByTestId('history-pad-top')).not.toHaveAttribute('aria-rowindex', /.*/);
  expect(h.pageErrors).toEqual([]);
});

test('j past the last rendered row reveals and focuses the next; End and Home reach the loaded ends', async ({ page }) => {
  const all = gen(N);
  const h = await serve(page, all, N);
  await open(page, N);
  const lastId = await rows(page).last().evaluate((el) => {
    (el as HTMLElement).focus({ preventScroll: true });
    return el.getAttribute('data-id')!;
  });
  const i = all.findIndex((r) => r.id === lastId);
  await page.keyboard.press('j');
  await expect.poll(() => focusedRowId(page)).toBe(all[i + 1].id);
  expect(await rowInView(page, all[i + 1].id)).toBe(true);
  await page.keyboard.press('End');
  await expect.poll(() => focusedRowId(page)).toBe(all[N - 1].id);
  expect(await rowInView(page, all[N - 1].id)).toBe(true);
  await page.keyboard.press('k');
  await expect.poll(() => focusedRowId(page)).toBe(all[N - 2].id);
  await page.keyboard.press('Home');
  await expect.poll(() => focusedRowId(page)).toBe(all[0].id);
  expect(await rowInView(page, all[0].id)).toBe(true);
  await page.keyboard.press('PageDown');
  await expect.poll(() => focusedRowId(page)).not.toBe(all[0].id);
  const afterPd = await focusedRowId(page);
  expect(all.findIndex((r) => r.id === afterPd)).toBeGreaterThan(1);
  expect(await rowInView(page, afterPd!)).toBe(true);
  await page.keyboard.press('PageUp');
  await expect.poll(() => focusedRowId(page)).toBe(all[0].id);
  expect(h.pageErrors).toEqual([]);
});

test('a focused row wheeled out of view parks focus on the list, never on body, and returns', async ({ page }) => {
  const all = gen(N);
  const h = await serve(page, all, N);
  await open(page, N);
  await scrollListTo(page, 1_000 * (await rowH(page)));
  const fv = await firstVisible(page);
  const id = fv!.id;
  const i = all.findIndex((r) => r.id === id);
  await page.locator(`tr[data-testid="history-row"][data-id="${id}"]`).focus();
  expect(await focusedRowId(page)).toBe(id);

  const box = (await list(page).boundingBox())!;
  const ch = await list(page).evaluate((el) => el.clientHeight);
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.wheel(0, ch * 3);
  await expect.poll(() => page.locator(`tr[data-testid="history-row"][data-id="${id}"]`).count()).toBe(0);
  await frames(page);
  expect(await activeTestId(page)).toBe('history-list');

  await page.mouse.wheel(0, -ch * 3);
  await expect.poll(() => focusedRowId(page)).toBe(id);

  // Out again; `j` on the list acts on the parked row: its successor is focused and revealed.
  await page.mouse.wheel(0, ch * 3);
  await expect.poll(() => activeTestId(page)).toBe('history-list');
  await page.keyboard.press('j');
  await expect.poll(() => focusedRowId(page)).toBe(all[i + 1].id);
  expect(await rowInView(page, all[i + 1].id)).toBe(true);
  expect(h.pageErrors).toEqual([]);
});

test('a checked row stays checked and counted outside the window; select-all takes every loaded row', async ({ page }) => {
  const all = gen(N);
  const h = await serve(page, all, N);
  await open(page, N);
  const box = page.locator(`tr[data-testid="history-row"][data-id="${all[2].id}"] input[type="checkbox"]`);
  await box.check();
  await expect(page.getByTestId('history-count')).toContainText(`${N} loaded · 1 selected`);
  await scrollListTo(page, 2_000 * (await rowH(page)));
  await expect(page.locator(`tr[data-testid="history-row"][data-id="${all[2].id}"]`)).toHaveCount(0);
  await expect(page.getByTestId('history-count')).toContainText('1 selected');
  await scrollListTo(page, 0);
  await expect(box).toBeChecked();
  await expect(page.getByTestId('history-count')).toContainText(`${N} loaded · 1 selected`);
  await page.getByTestId('select-all').check();
  await expect(page.getByTestId('history-count')).toContainText(`${N} loaded · ${N} selected`);
  await scrollListTo(page, 4_000 * (await rowH(page)));
  for (const b of await rows(page).locator('input[type="checkbox"]').all()) await expect(b).toBeChecked();
  expect(h.pageErrors).toEqual([]);
});

test('a burst of live rows keeps the anchored row in place mid-list; at the top new rows show', async ({ page }) => {
  const all = gen(N);
  const h = await serve(page, all, N);
  await open(page, N);
  await expect.poll(() => h.sockets.length).toBeGreaterThan(0);
  const ws = h.sockets[h.sockets.length - 1];
  await scrollListTo(page, 2_500 * (await rowH(page)) + 7);
  const before = await firstVisible(page);
  expect(before).not.toBeNull();
  let seq = 1;
  for (let i = 0; i < 300; i++) ws.send(JSON.stringify({ type: 'received', id: `live${String(i).padStart(4, '0')}`, seq: seq++ }));
  await expect(page.getByTestId('history-count')).toContainText(`${N + 300} loaded`);
  await frames(page);
  const after = await firstVisible(page);
  expect(after!.id).toBe(before!.id);
  expect(Math.abs(after!.offset - before!.offset)).toBeLessThanOrEqual(1);

  // Following the head: at scrollTop 0 the newest row is the first row.
  await scrollListTo(page, 0);
  for (let i = 0; i < 5; i++) ws.send(JSON.stringify({ type: 'received', id: `head${i}`, seq: seq++ }));
  await expect(page.getByTestId('history-count')).toContainText(`${N + 305} loaded`);
  await expect(rows(page).first()).toHaveAttribute('data-id', 'head4');
  expect(await list(page).evaluate((el) => el.scrollTop)).toBe(0);
  expect(h.pageErrors).toEqual([]);
});

test('scrolling near the end auto-loads; Load more still works from the keyboard', async ({ page }) => {
  const all = gen(N);
  const h = await serve(page, all, 100);
  await open(page, 100);
  expect(h.afters).toEqual([null]);
  await scrollListTo(page, 1e9);
  await expect.poll(() => h.afters.length).toBe(2);
  expect(h.afters[1]).toBe(all[99].id);
  await expect(page.getByTestId('history-count')).toContainText('200 loaded');

  // Keyboard path: from the last rendered checkbox, Tab reaches Load more.
  await rows(page).last().locator('input[type="checkbox"]').evaluate((el) => (el as HTMLElement).focus({ preventScroll: true }));
  await page.keyboard.press('Tab');
  await expect(page.getByTestId('load-more')).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(page.getByTestId('history-count')).toContainText('300 loaded');
  expect(h.afters[2]).toBe(all[199].id);
  expect(h.pageErrors).toEqual([]);
});

test('at the 50,000-row cap paging stops with "Limit reached" and a note', async ({ page }) => {
  test.setTimeout(180_000);
  const all = gen(50_100);
  const h = await serve(page, all, 5_000);
  await open(page, 5_000);
  for (let i = 0; i < 40; i++) {
    if ((await page.getByTestId('load-more').textContent())?.trim() === 'Limit reached') break;
    const n = await loadedCount(page);
    await scrollListTo(page, 1e9);
    await expect.poll(() => loadedCount(page)).toBeGreaterThan(n);
  }
  await expect(page.getByTestId('load-more')).toHaveText('Limit reached');
  await expect(page.getByTestId('load-more')).toBeDisabled();
  await expect(page.getByTestId('history-cap-note')).toBeVisible();
  await expect(page.getByTestId('history-cap-note')).toContainText('50,000 rows loaded');
  await expect(page.getByTestId('history-count')).toContainText('50000 loaded');
  // At the cap the total is known.
  await expect(list(page).locator('table')).toHaveAttribute('aria-rowcount', '50001');
  const pages = h.afters.length;
  await scrollListTo(page, 0);
  await scrollListTo(page, 1e9);
  await frames(page);
  expect(h.afters.length).toBe(pages);
  await expectBounded(page);
  expect(h.pageErrors).toEqual([]);
});

test('every rendered row has the same height, even with a 300-char host and 10 tags', async ({ page }) => {
  const long = 'a'.repeat(290) + '.example';
  const all = gen(200, 'r', (i) =>
    i % 3 === 0
      ? {
          host: long,
          path: `/${'x/'.repeat(150)}\n<b>multi\nline</b>`,
          tags: Array.from({ length: 10 }, (_, k) => `tag-${k}-${'t'.repeat(20)}`),
          method: 'OPTIONS',
          state: 'received',
        }
      : {},
  );
  const h = await serve(page, all, 200);
  await open(page, 200);
  for (const vp of [{ width: 1440, height: 900 }, { width: 1280, height: 720 }, { width: 1900, height: 1000 }]) {
    await page.setViewportSize(vp);
    await frames(page);
    const rh = await rowH(page);
    const heights = await rows(page).evaluateAll((els) => els.map((e) => e.getBoundingClientRect().height));
    expect(heights.length).toBeGreaterThan(5);
    for (const x of heights) expect(Math.abs(x - rh)).toBeLessThanOrEqual(0.5);
  }
  // The window custom properties are plain pixel lengths.
  await scrollListTo(page, 100 * (await rowH(page)));
  const props = await list(page).evaluate((el) => [
    el.style.getPropertyValue('--vt-before'),
    el.style.getPropertyValue('--vt-after'),
  ]);
  for (const p of props) expect(p).toMatch(/^\d+(\.\d+)?px$/);
  expect(h.pageErrors).toEqual([]);
});
