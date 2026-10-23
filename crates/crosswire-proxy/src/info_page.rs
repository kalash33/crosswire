//! The page served at `http://crosswire/`: what Crosswire is and its CA.

use bytes::{Bytes, BytesMut};

use crate::ProxyDeps;

/// The info page. Static: no scripts, no event handlers, no absolute URLs.
const INFO_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Crosswire</title>
<style>
body { font-family: system-ui, sans-serif; max-width: 40rem; margin: 2rem auto; padding: 0 1rem; line-height: 1.5; }
code { background: #eee; padding: 0 .2em; }
</style>
</head>
<body>
<h1>Crosswire</h1>
<p>Crosswire is an open-source web security testing proxy. This browser is
talking to it. To see HTTPS traffic, the browser must trust the Crosswire
certificate authority (CA) that signs the certificates it presents.</p>
<h2>Download the CA certificate</h2>
<ul>
<li><a href="/ca.der">/ca.der</a> &mdash; DER, for most operating system and browser certificate stores</li>
<li><a href="/ca.pem">/ca.pem</a> &mdash; PEM, for command-line tools and Firefox</li>
<li><a href="/ca.spki">/ca.spki</a> &mdash; SHA-256 of the CA public key (base64), for Chromium's <code>--ignore-certificate-errors-spki-list</code></li>
</ul>
<h2>Install it</h2>
<p>Import the certificate as a trusted authority for websites, then reload
this page over the proxy. Step-by-step instructions for each operating system
and browser are in <code>docs/ca-install.md</code> in the Crosswire source.</p>
<p>The CA is unique to this installation. Trust it only on machines you test
from, and remove it when you are done.</p>
</body>
</html>
"#;

const NOT_FOUND: &str = "Not found. Crosswire serves /, /ca.der, /ca.pem and /ca.spki here.";

/// The response for a local request with `method` and `path`.
pub(crate) fn respond(method: &[u8], path: &str, deps: &ProxyDeps) -> Bytes {
    if method != b"GET" {
        return response(
            404,
            "Not Found",
            "text/plain; charset=utf-8",
            NOT_FOUND.as_bytes(),
        );
    }
    match path {
        "/" => response(200, "OK", "text/html; charset=utf-8", INFO_HTML.as_bytes()),
        "/ca.der" => response(200, "OK", "application/pkix-cert", &deps.ca_der),
        "/ca.pem" => response(200, "OK", "application/x-pem-file", deps.ca_pem.as_bytes()),
        "/ca.spki" => {
            let body = format!("{}\n", deps.ca_spki);
            response(200, "OK", "text/plain; charset=utf-8", body.as_bytes())
        }
        _ => response(
            404,
            "Not Found",
            "text/plain; charset=utf-8",
            NOT_FOUND.as_bytes(),
        ),
    }
}

fn response(status: u16, reason: &str, content_type: &str, body: &[u8]) -> Bytes {
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nX-Content-Type-Options: nosniff\r\nContent-Security-Policy: default-src 'none'; style-src 'unsafe-inline'\r\nReferrer-Policy: no-referrer\r\n\r\n",
        body.len()
    );
    let mut out = BytesMut::with_capacity(head.len() + body.len());
    out.extend_from_slice(head.as_bytes());
    out.extend_from_slice(body);
    out.freeze()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_has_no_scripts_or_handlers_or_absolute_urls() {
        let lower = INFO_HTML.to_ascii_lowercase();
        assert!(!lower.contains("<script"));
        // No `on…=` attribute anywhere (inline event handlers).
        for (i, _) in lower.match_indices(" on") {
            let rest = &lower[i + 3..];
            let name_len = rest.bytes().take_while(u8::is_ascii_alphabetic).count();
            assert_ne!(rest.as_bytes().get(name_len), Some(&b'='), "inline handler");
        }
        assert!(!lower.contains("http://") && !lower.contains("https://"));
        assert!(lower.contains("docs/ca-install.md"));
        for link in ["/ca.der", "/ca.pem", "/ca.spki"] {
            assert!(INFO_HTML.contains(&format!("href=\"{link}\"")));
        }
    }
}
