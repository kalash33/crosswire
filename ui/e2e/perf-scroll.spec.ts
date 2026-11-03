import { test, expect } from '@playwright/test';
import { OVERSCAN } from '../src/lib/components/virtual-window';

// §13 UI performance gate: against a REAL `crosswire serve` on a seeded
// synthetic project (scripts/ui-perf.sh), scroll the history view for ~10 s
// while collecting requestAnimationFrame deltas in-page, then apply a host
// filter and time input -> filtered rows rendered.
//
// The history list is virtualised and auto-loads the next keyset page as the
// window nears the loaded end, so the driver only sends wheel events; pages
// loaded are counted from the network (`GET /api/v1/exchanges?…after=…`).
//
// ASSERTING when CROSSWIRE_UI_PERF_ASSERT=1 (scripts/ui-perf.sh sets it for
// <= 100k rows, so `just ui-perf` and CI assert): max frame <= 32 ms, filter
// < 300 ms, DOM rows <= 2 x visible + OVERSCAN (spec §3.3.2), at least one
// auto-loaded page and >= 1,000 rows loaded. Above 100k (the maintainer 1M run) it is
// REPORT-ONLY. The SUMMARY line is always printed and the gross-regression
// floors always assert. Excluded from the default suite (see playwright.config.ts).

// The page is already signed in: the config's global setup exchanged the
// single-use bootstrap token once and loads that session (storageState) into
// every attempt, including a retry.
const API = requireEnv('CROSSWIRE_API_URL');
const ROWS = process.env.CROSSWIRE_UI_PERF_ROWS ?? 'unknown';
const ASSERT = process.env.CROSSWIRE_UI_PERF_ASSERT === '1';
const SCROLL_MS = 10_000;
const MAX_FRAME_MS = 32;
const MAX_FILTER_MS = 300;
const MIN_LOADED_ROWS = 1_000;
// The synthetic generator's one-in-eight host (crosswire-store synthetic::HOST_X).
const FILTER_HOST = 'api.example.com';

function requireEnv(name: string): string {
  const v = process.env[name];
  if (!v) throw new Error(`missing env ${name} (run via scripts/ui-perf.sh)`);
  return v;
}

test('history scroll frames and filter latency at scale', async ({ page }) => {
  // Keyset page requests (`after=` present) are the auto-loads; the first page
  // and the filter's query carry no cursor.
  let pageRequests = 0;
  let counting = true;
  page.on('request', (req) => {
    if (!counting || req.method() !== 'GET') return;
    const url = new URL(req.url());
    if (url.pathname === '/api/v1/exchanges' && url.searchParams.has('after')) pageRequests += 1;
  });

  await page.goto(`${API}/`);
  await expect(page.getByTestId('history-view')).toBeVisible();
  await expect(page.getByTestId('history-row').first()).toBeVisible({ timeout: 30_000 });
  // Settle before measuring, so mount and first-page frames are not counted:
  // the first page fetched and the app idle on the network, then two frames.
  // Page fetches triggered by the scroll still fall inside the window.
  await page.waitForLoadState('networkidle');
  await page.evaluate(
    () => new Promise<void>((r) => requestAnimationFrame(() => requestAnimationFrame(() => r()))),
  );

  // ---- 10 s scroll, frames collected PASSIVELY in-page ----
  // The in-page collector only records rAF timestamps; it reads no layout. The
  // scroll is driven from outside by real wheel events only (the list
  // auto-loads), so nothing the harness does forces a reflow inside the
  // measured window.
  await page.evaluate(() => {
    const w = window as unknown as { __frames: number[]; __run: boolean };
    w.__frames = [];
    w.__run = true;
    let last = performance.now();
    const tick = (now: number) => {
      w.__frames.push(now - last);
      last = now;
      if (w.__run) requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  });
  // history-list is the scroll region, so aim the wheel at its centre.
  const box = await page.getByTestId('history-list').boundingBox();
  if (!box) throw new Error('history-list has no layout box');
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  const startMs = Date.now();
  while (Date.now() - startMs < SCROLL_MS) {
    await page.mouse.wheel(0, 300);
    await page.waitForTimeout(16);
  }
  // Stop the collector first; everything after this reads layout freely.
  const deltas = await page.evaluate(() => {
    const w = window as unknown as { __frames: number[]; __run: boolean };
    w.__run = false;
    return w.__frames;
  });
  counting = false;
  const pages = pageRequests;
  const scroll = await page.evaluate(() => {
    const list = document.querySelector<HTMLElement>('[data-testid="history-list"]')!;
    const row = document.querySelector('tr[data-testid="history-row"]');
    const rowH = row?.getBoundingClientRect().height ?? 0;
    const count = document.querySelector('[data-testid="history-count"]')?.textContent ?? '';
    return {
      domRows: document.querySelectorAll('[data-testid="history-row"]').length,
      domNodes: document.getElementsByTagName('*').length,
      visibleRows: rowH > 0 ? Math.ceil(list.clientHeight / rowH) : 0,
      // Tolerates thousands separators ("3,600", "3 600", "3.600").
      loadedRows: Number((/(\d[\d,.\u00a0\u202f ]*) loaded/.exec(count)?.[1] ?? '0').replace(/\D/g, '')),
    };
  });

  // The first delta spans setup, not a frame.
  const frames = deltas.slice(1);
  const maxFrame = Math.max(...frames);
  const sorted = [...frames].sort((a, b) => a - b);
  const p99 = sorted[Math.floor(sorted.length * 0.99)] ?? 0;
  // Index (in `frames`) and duration of every frame over the target.
  const slow = frames.flatMap((d, i) => (d > MAX_FRAME_MS ? [`${i}:${d.toFixed(1)}`] : []));
  const over32 = slow.length;

  // ---- filter: input -> filtered rows rendered ----
  // Back to the top of the scroll region (the window does not scroll).
  await page.evaluate(() => {
    document.querySelector('[data-testid="history-list"]')!.scrollTop = 0;
  });
  await page.getByTestId('filter-host').fill(FILTER_HOST);
  const filterMs = await page.evaluate(async (host) => {
    const rowsAreFiltered = () => {
      const rows = document.querySelectorAll('[data-testid="history-row"]');
      if (rows.length === 0) return false;
      return Array.from(rows).every((r) => (r.textContent ?? '').includes(host));
    };
    const apply = document.querySelector<HTMLButtonElement>('[data-testid="filter-apply"]')!;
    const t0 = performance.now();
    apply.click();
    // Rows rendered = every row is the filtered host (the old unfiltered rows
    // are replaced). Poll each frame; the end time is the frame that paints it.
    for (;;) {
      await new Promise((r) => requestAnimationFrame(r));
      if (rowsAreFiltered()) return performance.now() - t0;
      if (performance.now() - t0 > 30_000) return Infinity;
    }
  }, FILTER_HOST);

  // The acceptance bound (spec §3.3.2). The exact window size (visible +
  // 2 x OVERSCAN) is pinned by the vitest window math and history-virtual.spec.
  const domBound = 2 * scroll.visibleRows + OVERSCAN;
  console.log(
    `SUMMARY ui_perf rows=${ROWS} assert=${ASSERT ? 1 : 0} max_frame_ms=${maxFrame.toFixed(1)} ` +
      `p99_frame_ms=${p99.toFixed(1)} frames=${frames.length} frames_over_32ms=${over32} ` +
      `slow_frames=${slow.length ? slow.join(',') : 'none'} ` +
      `pages_loaded=${pages} loaded_rows=${scroll.loadedRows} dom_rows=${scroll.domRows} ` +
      `visible_rows=${scroll.visibleRows} dom_row_bound=${domBound} dom_nodes=${scroll.domNodes} ` +
      `filter_ms=${filterMs.toFixed(1)}`,
  );

  // Gross-regression floors: always on, at every size.
  expect(Number.isFinite(filterMs), 'filtered rows never rendered').toBe(true);
  expect(maxFrame, 'gross regression: a frame took >= 2 s').toBeLessThan(2000);
  expect(filterMs, 'gross regression: filter took >= 5 s').toBeLessThan(5000);

  if (!ASSERT) return;
  // §13 targets (spec 2026-11-02 §3.3.1). Soft, so one run reports every miss.
  expect.soft(maxFrame, `§13: a frame over ${MAX_FRAME_MS} ms during the scroll`).toBeLessThanOrEqual(MAX_FRAME_MS);
  expect.soft(filterMs, `§13: filter not applied within ${MAX_FILTER_MS} ms`).toBeLessThan(MAX_FILTER_MS);
  expect.soft(scroll.visibleRows, 'no measurable row height after the scroll').toBeGreaterThan(0);
  expect.soft(scroll.domRows, 'no history rows rendered after the scroll').toBeGreaterThan(0);
  expect
    .soft(scroll.domRows, `DOM rows not bounded by 2 x visible (${scroll.visibleRows}) + OVERSCAN (${OVERSCAN})`)
    .toBeLessThanOrEqual(domBound);
  expect.soft(pages, 'auto-load never fetched a page during the scroll').toBeGreaterThanOrEqual(1);
  expect.soft(scroll.loadedRows, 'the scroll did not go deep').toBeGreaterThanOrEqual(MIN_LOADED_ROWS);
});
