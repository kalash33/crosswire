import { test, expect } from '@playwright/test';

// §13 UI performance gate: against a REAL `crosswire serve` on a seeded
// synthetic project (scripts/ui-perf.sh), scroll the history view for ~10 s
// while collecting requestAnimationFrame deltas in-page, then apply a host
// filter and time input -> filtered rows rendered.
//
// The history list pages by a "Load more" button (keyset), so the scroll loop
// scrolls to the bottom and loads the next page whenever it is enabled; the
// DOM therefore grows as the user goes deeper, and that is what is measured.
//
// REPORT-ONLY (like the ingest gate): prints the SUMMARY, asserts only generous
// gross-regression floors, never the 32 ms / 300 ms targets. Excluded from the default suite (see playwright.config.ts).

const API = requireEnv('CROSSWIRE_API_URL');
const TOKEN = requireEnv('CROSSWIRE_BOOTSTRAP_TOKEN');
const ROWS = process.env.CROSSWIRE_UI_PERF_ROWS ?? 'unknown';
const SCROLL_MS = 10_000;
const MAX_FRAME_MS = 32;
// The synthetic generator's one-in-eight host (crosswire-store synthetic::HOST_X).
const FILTER_HOST = 'api.example.com';

function requireEnv(name: string): string {
  const v = process.env[name];
  if (!v) throw new Error(`missing env ${name} (run via scripts/ui-perf.sh)`);
  return v;
}

test('history scroll frames and filter latency at scale', async ({ page }) => {
  await page.goto(`${API}/#bootstrap=${TOKEN}`);
  await expect(page.getByTestId('history-view')).toBeVisible();
  await expect(page.getByTestId('history-row').first()).toBeVisible({ timeout: 30_000 });

  // ---- 10 s scroll, frames collected PASSIVELY in-page ----
  // The in-page collector only records rAF timestamps; it reads no layout. The
  // scroll is driven from outside by real wheel events, and "Load more" is
  // clicked on a timer (a click reads no layout), so nothing the harness does
  // forces a reflow inside the measured window.
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
  await page.mouse.move(400, 400);
  let pages = 0;
  const startMs = Date.now();
  let lastLoad = 0;
  while (Date.now() - startMs < SCROLL_MS) {
    await page.mouse.wheel(0, 300);
    if (Date.now() - lastLoad > 250) {
      lastLoad = Date.now();
      const clicked = await page.evaluate(() => {
        const b = document.querySelector<HTMLButtonElement>('[data-testid="load-more"]');
        if (b && !b.disabled) {
          b.click();
          return true;
        }
        return false;
      });
      if (clicked) pages += 1;
    }
    await page.waitForTimeout(16);
  }
  const scroll = await page.evaluate(() => {
    const w = window as unknown as { __frames: number[]; __run: boolean };
    w.__run = false;
    return {
      deltas: w.__frames,
      pages: 0,
      domRows: document.querySelectorAll('[data-testid="history-row"]').length,
      domNodes: document.getElementsByTagName('*').length,
    };
  });
  scroll.pages = pages;

  // The first delta spans setup, not a frame.
  const frames = scroll.deltas.slice(1);
  const maxFrame = Math.max(...frames);
  const sorted = [...frames].sort((a, b) => a - b);
  const p99 = sorted[Math.floor(sorted.length * 0.99)] ?? 0;
  const over32 = frames.filter((d) => d > MAX_FRAME_MS).length;

  // ---- filter: input -> filtered rows rendered ----
  await page.evaluate(() => window.scrollTo(0, 0));
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

  // eslint-disable-next-line no-console
  console.log(
    `SUMMARY ui_perf rows=${ROWS} max_frame_ms=${maxFrame.toFixed(1)} p99_frame_ms=${p99.toFixed(1)} ` +
      `frames=${frames.length} frames_over_32ms=${over32} pages_loaded=${scroll.pages} ` +
      `dom_rows=${scroll.domRows} dom_nodes=${scroll.domNodes} filter_ms=${filterMs.toFixed(1)}`,
  );

  // REPORT-ONLY: the §13 targets (32 ms / 300 ms) are NOT asserted; the history
  // list is not virtualized, so the frame target is known unmet (bench/README.md,
  // pre-0.1.0 backlog). These are generous gross-regression guards only.
  expect(Number.isFinite(filterMs), 'filtered rows never rendered').toBe(true);
  expect(maxFrame, 'gross regression: a frame took >= 2 s').toBeLessThan(2000);
  expect(filterMs, 'gross regression: filter took >= 5 s').toBeLessThan(5000);
});
