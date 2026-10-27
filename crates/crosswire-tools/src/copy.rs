//! Render a recorded request as a runnable `curl` command or as its raw
//! wire bytes.
//!
//! The renderers are server-side on purpose: the shell-quoting routine
//! ([`quote`]) is the one place a value is made safe for a shell, so it is a
//! single documented, testable function rather than UI string-building that a
//! later change could get subtly wrong. Rendering the output of
//! [`RedactionPolicy::redact_message`] cannot leak a secret the policy
//! removed, because these functions only ever read the bytes they are given.
//!
//! [`RedactionPolicy::redact_message`]: crosswire_model::RedactionPolicy::redact_message

use crosswire_model::{Body, Head, Message, StartLineSpans};

/// Which shell the quoted argument is destined for.
///
/// The two shells quote single-quoted strings differently (see [`quote`]), so
/// the caller picks the target up front.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Shell {
    /// A POSIX `sh`-family shell (bash, zsh, dash, ...).
    Posix,
    /// Windows PowerShell.
    PowerShell,
}

/// Quotes one argument so a shell passes it through as a single literal field.
///
/// ## Why this is total and safe
///
/// Both shells are handled by wrapping the whole argument in a single-quoted
/// string and dealing only with the embedded single quote (`'`, byte `0x27`):
///
/// * **POSIX** — inside a single-quoted string *no* character is special, not
///   even a backslash, so the only byte that can end the quoting is `'`
///   itself. Each `'` is emitted as `'\''`: close the quote, an escaped
///   literal quote, reopen the quote. Every other byte — spaces, `$`, `` ` ``,
///   `;`, newlines, globs — is copied verbatim and cannot be interpreted.
///   This is the POSIX.1 shell rule, so the result is a single field for any
///   input.
///
/// * **PowerShell** — inside a single-quoted string the only metacharacter is
///   `'`, and it is escaped by doubling it (`''`). Nothing else is special.
///
/// Because `0x27` is ASCII, it never appears inside a multi-byte UTF-8
/// sequence, so replacing it byte-for-byte is correct for any UTF-8 text.
/// The routine never fails: it copies bytes, and any input that is not valid
/// UTF-8 is rendered with the standard lossy replacement, still fully quoted.
#[must_use]
pub fn quote(arg: &[u8], shell: Shell) -> String {
    let mut out: Vec<u8> = Vec::with_capacity(arg.len() + 2);
    out.push(b'\'');
    for &b in arg {
        if b == b'\'' {
            match shell {
                // close quote, escaped literal quote, reopen quote.
                Shell::Posix => out.extend_from_slice(b"'\\''"),
                // doubled single quote inside the single-quoted string.
                Shell::PowerShell => out.extend_from_slice(b"''"),
            }
        } else {
            out.push(b);
        }
    }
    out.push(b'\'');
    String::from_utf8_lossy(&out).into_owned()
}

/// Whether the request's body is a blob reference whose bytes are not held in
/// process. The render functions stay pure (they never consult a pack), so a
/// caller that wants to flag the omission asks here.
#[must_use]
pub fn body_is_blob(req: &Message) -> bool {
    matches!(req.body(), Body::Blob { .. })
}

/// Renders `req` as a runnable `curl` command for `shell`.
///
/// The URL is `scheme://authority + request-target`, where the scheme is
/// `https` when the exchange was recorded over TLS (a negotiated TLS version
/// is present) and `http` otherwise, and the authority is the recorded
/// `target_addr`, with `:port` appended when the port is not the scheme
/// default. The request-target is taken verbatim from the start line; an
/// absolute-form target (one already containing `://`) is used as the whole
/// URL. The method is passed with `-X` unless it is `GET`. Each header is a
/// `-H 'Name: value'` in the recorded order. An inline body is passed with
/// `--data-binary`; a blob body contributes no bytes and instead appends a
/// `# body omitted (N bytes in pack)` comment line (see [`body_is_blob`]).
///
/// Output depends only on `req` and `shell`: no locale or environment.
#[must_use]
pub fn to_curl(req: &Message, shell: Shell) -> String {
    let RequestParts {
        method,
        target,
        headers,
    } = request_parts(req);
    let url = build_url(req, &target);

    let mut parts: Vec<String> = Vec::new();
    parts.push(String::from("curl"));
    if method != b"GET" {
        parts.push(String::from("-X"));
        parts.push(quote(&method, shell));
    }
    parts.push(quote(url.as_bytes(), shell));
    for (name, value) in &headers {
        let mut field = Vec::with_capacity(name.len() + value.len() + 2);
        field.extend_from_slice(name);
        field.extend_from_slice(b": ");
        field.extend_from_slice(value);
        parts.push(String::from("-H"));
        parts.push(quote(&field, shell));
    }

    match req.body() {
        Body::Inline { bytes, .. } => {
            parts.push(String::from("--data-binary"));
            parts.push(quote(bytes, shell));
            parts.join(" ")
        }
        Body::Blob { len, .. } => {
            let cmd = parts.join(" ");
            format!("{cmd}\n# body omitted ({len} bytes in pack)")
        }
        // No body (or a variant newer than this crate): just the command.
        _ => parts.join(" "),
    }
}

/// Renders `req` as its raw wire bytes: the head bytes followed by the inline
/// body bytes.
///
/// For an HTTP/1 head these are exactly the stored head bytes (including the
/// terminating empty line); a translated (`Normalized`) head is reconstructed
/// as `Name: value` lines, and a tunnel head has no bytes. A `Body::Blob`
/// holds no in-process bytes and so contributes nothing; a request with no
/// body yields the head bytes alone.
#[must_use]
pub fn to_raw(req: &Message) -> Vec<u8> {
    let mut out = head_bytes(req.head());
    if let Body::Inline { bytes, .. } = req.body() {
        out.extend_from_slice(bytes);
    }
    out
}

/// The request line and headers pulled out of a message for rendering, as
/// owned bytes.
struct RequestParts {
    method: Vec<u8>,
    target: Vec<u8>,
    headers: Vec<(Vec<u8>, Vec<u8>)>,
}

/// The method, request-target and headers of a request message as owned bytes.
///
/// For an HTTP/1 head these are sliced from the stored head bytes by their
/// spans. A translated head carries no start line, so the method defaults to
/// `GET` and the target to `/` (documented best effort); its header pairs are
/// used as-is. A tunnel head yields nothing.
fn request_parts(req: &Message) -> RequestParts {
    match req.head() {
        Head::Http1Raw { bytes, spans, .. } => {
            let slice = |s: crosswire_core::Span| bytes.get(s.range()).unwrap_or(&[]).to_vec();
            let (method, target) = match spans.start_line() {
                StartLineSpans::Request { method, target, .. } => (slice(*method), slice(*target)),
                // A response head (or a shape newer than this crate) is not a
                // request; render as a bare GET.
                _ => (b"GET".to_vec(), b"/".to_vec()),
            };
            let headers = spans
                .headers()
                .iter()
                .map(|h| (slice(h.name()), slice(h.value())))
                .collect();
            RequestParts {
                method,
                target,
                headers,
            }
        }
        Head::Normalized { map } => RequestParts {
            method: b"GET".to_vec(),
            target: b"/".to_vec(),
            headers: map
                .iter()
                .map(|(n, v)| (n.as_bytes().to_vec(), v.clone()))
                .collect(),
        },
        // A tunnel (or a head shape newer than this crate): nothing to render.
        _ => RequestParts {
            method: b"GET".to_vec(),
            target: b"/".to_vec(),
            headers: Vec::new(),
        },
    }
}

/// Builds the request URL from the connection facts and the request-target.
fn build_url(req: &Message, target: &[u8]) -> String {
    let conn = req.connection();
    let scheme = if conn.tls_version().is_some() {
        "https"
    } else {
        "http"
    };
    let target = String::from_utf8_lossy(target);
    // An absolute-form target already is the whole URL.
    if target.contains("://") {
        return target.into_owned();
    }
    let host = conn.target_addr();
    let port = conn.port();
    let default_port = if scheme == "https" { 443 } else { 80 };
    if port == 0 || port == default_port {
        format!("{scheme}://{host}{target}")
    } else {
        format!("{scheme}://{host}:{port}{target}")
    }
}

/// The raw head bytes for [`to_raw`].
fn head_bytes(head: &Head) -> Vec<u8> {
    match head {
        Head::Http1Raw { bytes, .. } => bytes.to_vec(),
        Head::Normalized { map } => {
            let mut out = Vec::new();
            for (name, value) in map {
                out.extend_from_slice(name.as_bytes());
                out.extend_from_slice(b": ");
                out.extend_from_slice(value);
                out.extend_from_slice(b"\r\n");
            }
            out.extend_from_slice(b"\r\n");
            out
        }
        // A tunnel (or newer head shape) has no reconstructable head bytes.
        _ => Vec::new(),
    }
}
