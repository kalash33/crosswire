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
// Report-only by default; CROSSWIRE_PERF_ASSERT=1 asserts max frame <= 32 ms
// and filter < 300 ms. Excluded from the default suite (see playwright.config.ts).

const API = requireEnv('CROSSWIRE_API_URL');
const TOKEN = requireEnv('CROSSWIRE_BOOTSTRAP_TOKEN');
const ROWS = process.env.CROSSWIRE_UI_PERF_ROWS ?? 'unknown';
const ASSERT = process.env.CROSSWIRE_PERF_ASSERT === '1';
const SCROLL_MS = 10_000;
const MAX_FRAME_MS = 32;
const MAX_FILTER_MS = 300;
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

  // ---- 10 s scroll, frames collected in-page ----
  const scroll = await page.evaluate(async (durationMs) => {
    const deltas: number[] = [];
    let last = performance.now();
    let running = true;
    const tick = (now: number) => {
      deltas.push(now - last);
      last = now;
      if (running) requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);

    const loadMore = document.querySelector<HTMLButtonElement>('[data-testid="load-more"]');
    const start = performance.now();
    let pages = 0;
    while (performance.now() - start < durationMs) {
      window.scrollBy(0, 400);
      const list = document.querySelector('[data-testid="history-list"]');
      if (list) list.scrollBy(0, 400);
      // Reaching the end of what is loaded: page in the next chunk.
      const nearBottom =
        window.innerHeight + window.scrollY >= document.documentElement.scrollHeight - 800;
      if (nearBottom && loadMore && !loadMore.disabled) {
        loadMore.click();
        pages += 1;
      }
      await new Promise((r) => requestAnimationFrame(r));
    }
    running = false;
    await new Promise((r) => requestAnimationFrame(r));
    return {
      deltas,
      pages,
      domRows: document.querySelectorAll('[data-testid="history-row"]').length,
      domNodes: document.getElementsByTagName('*').length,
    };
  }, SCROLL_MS);

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

  expect(Number.isFinite(filterMs), 'filtered rows never rendered').toBe(true);
  if (ASSERT) {
    expect(maxFrame, `max frame ${maxFrame.toFixed(1)} ms > ${MAX_FRAME_MS} ms`).toBeLessThanOrEqual(MAX_FRAME_MS);
    expect(filterMs, `filter ${filterMs.toFixed(1)} ms >= ${MAX_FILTER_MS} ms`).toBeLessThan(MAX_FILTER_MS);
  }
});
