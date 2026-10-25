import type { Page } from '@playwright/test';

// Execution detectors shared by the XSS gate specs. Register BEFORE any
// attacker-influenced data renders. Returns the collected violations; the
// caller asserts it is empty.
export function watch(page: Page, appOrigin: string): string[] {
  const violations: string[] = [];
  page.on('dialog', async (d) => {
    violations.push(`dialog(${d.type()}): ${d.message()}`);
    await d.dismiss();
  });
  page.on('request', (req) => {
    const u = new URL(req.url());
    if (!/^https?:$/.test(u.protocol)) return; // data:/blob:/about: carry no network egress
    if (u.origin !== appOrigin) violations.push(`off-origin request: ${req.url()}`);
    else if (!u.pathname.startsWith('/api/v1/') && !u.pathname.startsWith('/assets/') && u.pathname !== '/')
      violations.push(`unexpected same-origin request: ${req.url()}`);
  });
  page.on('requestfailed', (req) => {
    if (new URL(req.url()).origin !== appOrigin && /^https?:/.test(req.url()))
      violations.push(`off-origin request failed: ${req.url()}`);
  });
  page.on('framenavigated', (f) => {
    if (f === page.mainFrame() && new URL(f.url()).origin !== appOrigin)
      violations.push(`navigated away: ${f.url()}`);
  });
  page.on('popup', (p) => violations.push(`popup: ${p.url()}`));
  return violations;
}

// Live-DOM probe: did any payload become an element or set the execution flag?
export function liveProbe(page: Page) {
  return page.evaluate(() => ({
    pwned: (window as unknown as { __xss?: number }).__xss ?? 0,
    injected: document.querySelectorAll(
      '[data-testid="app-root"] script, iframe, object, embed, meta[http-equiv], base, svg[onload], [onmouseover], [onerror], [onfocus], a[href^="javascript:"], link[rel=stylesheet][href*="evil"]',
    ).length,
  }));
}
