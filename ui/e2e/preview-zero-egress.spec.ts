// §13 security gate #4: preview zero-egress corpus.
//
// Each hostile body is served to the real sandboxed preview iframe (the real
// ExchangeDetail markup) with the REAL deny-all preview headers, and the test
// asserts that NO request leaves the preview document: not to an external host
// and not to the localhost API/cookie origin. The CSP string is read from the
// server source so a header regression there cannot drift from this spec.
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { test, expect, type Page } from '@playwright/test';

const ID = '01HZZZZZZZZZZZZZZZZZZZZZSHP';
const b64 = (s: string) => Buffer.from(s, 'utf8').toString('base64');
const HEAD = 'HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n';
const msg = (body: unknown) => ({
  kind: 'http1-raw', fidelity: 'raw', diagnostics: [],
  head: { type: 'http1-raw', bytes: b64(HEAD), parser_version: 1 }, body,
});

const routeSrc = readFileSync(
  fileURLToPath(new URL('../../crates/crosswire-api/src/routes/preview.rs', import.meta.url)),
  'utf8',
);
const SERVER_CSP = /const PREVIEW_CSP: &str = "([^"]+)";/.exec(routeSrc)?.[1];

const PREVIEW_HEADERS = {
  'content-security-policy': "sandbox; default-src 'none'; img-src data: blob:",
  'referrer-policy': 'no-referrer',
  'x-content-type-options': 'nosniff',
};

const EXT = 'http://evil.example';
const API = 'http://localhost:7777/api/v1/exchanges';
const MARK = '<p id="m">corpus-loaded</p>';
// Scripts are inert under the sandbox, but are included so a relaxed sandbox is caught.
const AUTO = `<script>document.getElementById('a')&&document.getElementById('a').click();document.getElementById('f')&&document.getElementById('f').submit();</script>`;

const VECTORS: Record<string, string> = {
  'img src': `<img src="${EXT}/i.png"><img src="${API}">`,
  'meta refresh': `<meta http-equiv="refresh" content="0;url=${EXT}/r">`,
  'auto-clicked anchor': `<a id="a" href="${EXT}/a">x</a>${AUTO}`,
  'auto-submitted form': `<form id="f" action="${EXT}/f" method="post"><input name="x" value="1"></form>${AUTO}`,
  'css background url': `<div style="background:url(${EXT}/bg.png);width:9px;height:9px"></div><style>body{background:url(${EXT}/bg2.png)}</style>`,
  'css @import': `<style>@import url("${EXT}/x.css");</style>`,
  'link stylesheet': `<link rel="stylesheet" href="${EXT}/s.css">`,
  'link prefetch': `<link rel="prefetch" href="${EXT}/p"><link rel="preload" as="image" href="${EXT}/pl.png">`,
  'iframe src': `<iframe src="${EXT}/frame"></iframe><iframe src="${API}"></iframe>`,
  'object data': `<object data="${EXT}/o.swf"></object><embed src="${EXT}/e">`,
  'video poster': `<video poster="${EXT}/poster.png" src="${EXT}/v.mp4"></video>`,
  'svg use href': `<svg><use href="${EXT}/s.svg#a"></use></svg><svg><image href="${EXT}/img.svg"/></svg>`,
};

async function load(page: Page, html: string) {
  // A request that reaches the network layer is a leak. Requests the browser refuses itself
  // (CSP `csp`, or the unique-origin check `origin`) fail before the network layer, never invoke
  // the external route below, and are not egress. `reached` is the authoritative signal.
  const requests: { url: string; sub: boolean }[] = [];
  page.on('request', (r) => requests.push({ url: r.url(), sub: r.frame() !== page.mainFrame() }));
  const blocked = new Set<string>();
  const reached: string[] = []; // external requests that got past the browser to the network layer
  page.on('requestfailed', (r) => blocked.add(r.url()));
  await page.route('**/api/v1/counts', (r) => r.fulfill({ status: 200, json: { exchanges: 1 } }));
  await page.route(/\/api\/v1\/exchanges\/[^/?]+$/, (r) =>
    r.fulfill({ status: 200, json: { id: ID, request: msg({ type: 'none' }), response: msg({ type: 'inline', truncated_at: 3 }), scope_state: 'in', state: 'committed', tags: [], notes: '' } }),
  );
  await page.route(/\/body\/req$/, (r) => r.fulfill({ status: 404, json: { error: { code: 'not_found', message: 'x' } } }));
  await page.route(/\/body\/resp$/, (r) =>
    r.fulfill({ status: 200, contentType: 'application/octet-stream', headers: { 'x-crosswire-length': '3' }, body: Buffer.from('abc') }),
  );
  await page.route(/\/api\/v1\/exchanges(\?.*)?$/, (r) =>
    r.fulfill({ status: 200, json: { items: [{ id: ID, created_at: 1_700_000_000_000, method: 'GET', host: 'a.example', path: '/', status: 200, resp_len: 3, state: 'committed', tags: [] }], next_cursor: null } }),
  );
  await page.route('**/api/v1/exchanges/*/preview-token', (r) =>
    r.fulfill({ status: 200, json: { token: 't', path: `/preview/${ID}?token=t`, expires_in: 60 } }),
  );
  // Everything outside the app origin is answered 200 (a leak would be observed, not blocked by a failure).
  await page.route(/^http:\/\/(evil\.example|localhost:\d+)\//, (r) => { reached.push(r.request().url()); return r.fulfill({ status: 200, contentType: 'text/html', body: 'LEAK' }); });
  let previewResp: { headers: Record<string, string> } | undefined;
  await page.route('http://127.0.0.1:*/preview/**', async (r) => {
    await r.fulfill({ status: 200, contentType: 'text/html; charset=utf-8', headers: PREVIEW_HEADERS, body: `<!doctype html><html><head></head><body>${MARK}${html}</body></html>` });
  });
  page.on('response', async (resp) => {
    if (resp.url().includes('/preview/')) previewResp = { headers: await resp.allHeaders() };
  });

  await page.goto('/');
  await page.getByTestId('history-row').first().click();
  const baseline = requests.length;
  await page.getByTestId('preview-button').click();
  const frame = page.getByTestId('preview-frame');
  await expect(frame).toBeVisible();
  // Prove the hostile document really loaded, so zero-egress is not vacuous.
  await expect(page.frameLocator('[data-testid="preview-frame"]').locator('#m')).toHaveText('corpus-loaded');
  await page.waitForTimeout(1000);
  const leaked = [...reached, ...requests
    .slice(baseline)
    .filter((q) => !blocked.has(q.url))
    // The app's own same-origin API calls from the main frame are not preview egress.
    .filter((q) => q.sub || !q.url.startsWith(new URL(page.url()).origin))
    .map((q) => q.url)
    .filter((u) => !/^http:\/\/127\.0\.0\.1:\d+\/preview\//.test(u))];
  return { leaked, blocked: [...blocked], previewResp: () => previewResp };
}

test('the headers asserted here are the server\'s deny-all set', () => {
  expect(SERVER_CSP).toBe(PREVIEW_HEADERS['content-security-policy']);
  expect(routeSrc).toContain('no-referrer');
});

for (const [name, html] of Object.entries(VECTORS)) {
  test(`zero egress: ${name}`, async ({ page }) => {
    const { leaked, previewResp } = await load(page, html);
    const h = previewResp()?.headers ?? {};
    expect(h['content-security-policy']).toBe("sandbox; default-src 'none'; img-src data: blob:");
    expect(h['referrer-policy']).toBe('no-referrer');
    expect(h['set-cookie']).toBeUndefined();
    expect(leaked).toEqual([]);
    expect(new URL(page.url()).pathname).toBe('/'); // no top-level navigation either
  });
}
