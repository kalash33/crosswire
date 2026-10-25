import { test, expect, type Page, type Request } from '@playwright/test';

// No backend: Playwright serves the built bundle via `vite preview` and we mock
// every API call with page.route. The app authenticates off a 200 from
// /api/v1/counts (session.probe), then the Repeater view loads its tabs.
//
// The star of this file is the PASTE-RAW BYTE-ROUND-TRIP GATE: a request whose
// body carries the full 0x00..0xFF byte range is pasted (raw) into the editable
// ByteEditor and sent. On the wire `SendBody.raw` is STANDARD base64 of the
// request bytes, so the gate base64-decodes what the client posted and asserts
// it equals the pasted bytes EXACTLY — proving no corruption across the editable
// editor + raw send path + base64 wire.

const COUNTS = {
  bytes_inline: 0, bytes_packs: 0, committed: 0, error: 0, exchanges: 0,
  history: 0, intruder: 0, markers: 0, repeater: 0,
};

const EXCHANGE_ID = '01HZZZZZZZZZZZZZZZZZZZZREPE';
const RESP_BODY = 'RESP-OK-BODY-123'; // printable ASCII — asserted via cm-content

// A request whose body is every byte value 0x00..0xFF (as a Latin-1 string).
const REQ_HEAD = 'POST / HTTP/1.1\r\nHost: example.test\r\nContent-Length: 256\r\n\r\n';
function allBytesBody(): string {
  let s = '';
  for (let b = 0; b < 256; b++) s += String.fromCharCode(b);
  return s;
}
const FULL_RANGE_REQUEST = REQ_HEAD + allBytesBody();

// A request carrying Transfer-Encoding — assisted mode must show the
// Content-Length chip as "skipped: TE present" (RFC 7230 framing guard).
const TE_REQUEST =
  'POST / HTTP/1.1\r\nHost: example.test\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n';

function outcome() {
  return {
    exchange_id: EXCHANGE_ID,
    scope_state: 'in',
    changes: [],
    skipped: [],
    cert_override: false,
    ttfb_ms: 12,
    total_ms: 34,
    redirects: [],
  };
}

interface Harness {
  sends: Request[];
}

async function mockApi(page: Page): Promise<Harness> {
  const harness: Harness = { sends: [] };

  // A deterministic, in-page clipboard: the "Paste raw" button reads it via
  // navigator.clipboard.readText(). Set it from the test with __setClip().
  await page.addInitScript(() => {
    let clip = '';
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: {
        readText: async () => clip,
        writeText: async (t: string) => {
          clip = t;
        },
      },
    });
    (window as unknown as { __setClip: (t: string) => void }).__setClip = (t: string) => {
      clip = t;
    };
  });

  await page.route('**/api/v1/counts', (route) => route.fulfill({ status: 200, json: COUNTS }));

  // Repeater tab drafts: start empty; autosave PUT / DELETE are accepted.
  await page.route(/\/api\/v1\/repeater\/tabs$/, (route) =>
    route.fulfill({ status: 200, json: [] }),
  );
  await page.route(/\/api\/v1\/repeater\/tabs\/[^/]+$/, (route) =>
    route.fulfill({ status: 200, body: '' }),
  );

  // Send: record the posted body, return the outcome.
  await page.route(/\/api\/v1\/repeater\/send$/, (route) => {
    harness.sends.push(route.request());
    route.fulfill({ status: 200, json: outcome() });
  });

  // The response bytes for the outcome's exchange_id.
  await page.route(/\/api\/v1\/exchanges\/[^/]+\/body\/resp$/, (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/octet-stream',
      headers: { 'x-crosswire-length': String(RESP_BODY.length) },
      body: Buffer.from(RESP_BODY, 'latin1'),
    }),
  );
  await page.route(/\/api\/v1\/exchanges\/[^/]+\/body\/req$/, (route) =>
    route.fulfill({ status: 404, json: { error: { code: 'not_found', message: 'no body' } } }),
  );

  return harness;
}

async function openRepeater(page: Page): Promise<void> {
  await page.goto('/');
  await expect(page.getByTestId('app-shell')).toBeVisible();
  // Navigate via the nav entry (pushState SPA route — no server round-trip).
  await page.getByTestId('nav-repeater').click();
  await expect(page.getByTestId('repeater-view')).toBeVisible();
  // Opening the view always shows at least one tab.
  await expect(page.getByTestId('repeater-tab')).toHaveCount(1);
}

test('repeater: set conn, paste a raw request, send, render the result header and response', async ({
  page,
}) => {
  const harness = await mockApi(page);
  await openRepeater(page);

  // Connection settings.
  await page.getByTestId('conn-host').fill('example.test');
  await page.getByTestId('conn-port').fill('8443');

  // Raw mode: the bytes are sent untouched (no assisted chips).
  await page.getByTestId('mode-raw').click();
  await expect(page.getByTestId('assisted-chip')).toHaveCount(0);

  // Paste the full-range request (raw, Latin-1 1:1) and send.
  await page.evaluate(
    (t) => (window as unknown as { __setClip: (t: string) => void }).__setClip(t),
    FULL_RANGE_REQUEST,
  );
  await page.getByTestId('paste-raw').click();

  const sendPosted = page.waitForRequest(/\/api\/v1\/repeater\/send$/);
  await page.getByTestId('send').click();
  await sendPosted;

  // ---- PASTE-RAW BYTE-ROUND-TRIP GATE --------------------------------------
  const sent = harness.sends.at(-1)!;
  const body = JSON.parse(sent.postData() ?? '{}') as {
    raw: string;
    host: string;
    port: number;
    mode?: string;
  };
  expect(body.host).toBe('example.test');
  expect(body.port).toBe(8443);
  expect(body.mode).toBe('raw');
  // `raw` is standard base64 of the request bytes; decode and compare byte-exact.
  const decoded = Buffer.from(body.raw, 'base64');
  const expected = Buffer.from(FULL_RANGE_REQUEST, 'latin1');
  expect(decoded.equals(expected)).toBe(true);
  expect(decoded.length).toBe(expected.length);

  // ---- Result header -------------------------------------------------------
  await expect(page.getByTestId('result-header')).toBeVisible();
  await expect(page.getByTestId('result-ttfb')).toContainText('12');
  await expect(page.getByTestId('result-total')).toContainText('34');
  await expect(page.getByTestId('result-scope-state')).toContainText('in');

  // ---- Response bytes rendered (read-only editor) --------------------------
  const resp = page.getByTestId('result-resp-body');
  await expect(resp).toBeVisible();
  await expect(resp.locator('.cm-content')).toContainText(RESP_BODY);
});

test('repeater: assisted mode shows the Content-Length chip as "skipped: TE present"', async ({
  page,
}) => {
  await mockApi(page);
  await openRepeater(page);

  // Assisted is the default mode; paste a TE request.
  await expect(page.getByTestId('mode-assisted')).toBeVisible();
  await page.evaluate(
    (t) => (window as unknown as { __setClip: (t: string) => void }).__setClip(t),
    TE_REQUEST,
  );
  await page.getByTestId('paste-raw').click();

  // The assisted surface shows the TE-guarded Content-Length chip.
  await expect(
    page.getByTestId('assisted-chip').filter({ hasText: 'skipped: TE present' }),
  ).toBeVisible();
});
