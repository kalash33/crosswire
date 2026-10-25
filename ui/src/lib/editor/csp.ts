// CSP nonce discovery.
//
// Under the production CSP (style-src 'self' 'nonce-<n>') the server injects a
// per-response <meta name="csp-nonce" content="..."> (Task 1). CodeMirror
// injects a <style> element at runtime, which that CSP blocks unless it carries
// the matching nonce. We read the nonce here and hand it to CodeMirror via its
// EditorView.cspNonce facet.
//
// In vite dev/preview there is no server CSP and no such meta, so this returns
// null and the editor degrades gracefully (styles are allowed anyway).
export function readCspNonce(): string | null {
  const el = document.querySelector('meta[name="csp-nonce"]');
  const nonce = el?.getAttribute('content');
  return nonce && nonce.length > 0 ? nonce : null;
}
