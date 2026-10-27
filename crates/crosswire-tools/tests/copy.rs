//! Tests for the curl / raw-bytes renderers and the shell-quoting routine.
//!
//! The quoting tests assert structurally — they parse the quoted string with a
//! local POSIX word-splitter rather than executing any shell.

use bytes::Bytes;
use crosswire_h1::{Parser, ParserConfig, Progress, RawMessage};
use crosswire_model::{Body, ConnectionInfo, Message, RedactionPolicy};
use crosswire_tools::{body_is_blob, quote, to_curl, to_raw, Shell};

/// Parses `head` bytes into a `RawMessage` (head only).
fn parse_head(head: &[u8]) -> RawMessage {
    let mut p = Parser::request(&ParserConfig::default());
    match p.feed(head).expect("head parses").event {
        Progress::Head(m) => m,
        other => panic!("expected head, got {other:?}"),
    }
}

/// A plaintext request message from head bytes, a body, host and port.
fn request(head: &[u8], body: Body, host: &str, port: u16) -> Message {
    Message::http1_raw(&parse_head(head), body, ConnectionInfo::new(host, port))
}

/// A TLS request message (so the rendered scheme is `https`).
fn tls_request(head: &[u8], body: Body, host: &str, port: u16) -> Message {
    let conn = ConnectionInfo::new(host, port).with_tls("TLSv1.3", "TLS_AES_128_GCM_SHA256");
    Message::http1_raw(&parse_head(head), body, conn)
}

/// Re-splits a POSIX command line into fields, honouring single quotes and
/// backslash escapes. No shell is executed. Only what [`quote`] can emit is
/// handled: single-quoted runs, a backslash-escaped character, and literal
/// bytes; space and tab separate fields.
fn posix_resplit(s: &[u8]) -> Vec<Vec<u8>> {
    let mut fields: Vec<Vec<u8>> = Vec::new();
    let mut cur: Vec<u8> = Vec::new();
    let mut started = false;
    let mut in_single = false;
    let mut i = 0;
    while i < s.len() {
        let b = s[i];
        if in_single {
            if b == b'\'' {
                in_single = false;
            } else {
                cur.push(b);
            }
            i += 1;
            continue;
        }
        match b {
            b'\'' => {
                in_single = true;
                started = true;
                i += 1;
            }
            b'\\' if i + 1 < s.len() => {
                cur.push(s[i + 1]);
                started = true;
                i += 2;
            }
            b' ' | b'\t' => {
                if started {
                    fields.push(core::mem::take(&mut cur));
                    started = false;
                }
                i += 1;
            }
            other => {
                cur.push(other);
                started = true;
                i += 1;
            }
        }
    }
    if started {
        fields.push(cur);
    }
    fields
}

#[test]
fn posix_quote_is_total() {
    let raw = b"a'b$(id)`\nc;d";
    let quoted = quote(raw, Shell::Posix);
    let expected = "'a'\\''b$(id)`\nc;d'";
    assert_eq!(quoted, expected);
    // Re-splitting the quoted string under POSIX word rules yields exactly the
    // original bytes as one field.
    let fields = posix_resplit(quoted.as_bytes());
    assert_eq!(fields, vec![raw.to_vec()]);
}

#[test]
fn powershell_quote_doubles_quotes() {
    let quoted = quote(b"a'b", Shell::PowerShell);
    assert_eq!(quoted, "'a''b'");
}

#[test]
fn curl_get_has_no_data_flag() {
    let req = request(
        b"GET /search HTTP/1.1\r\nHost: example.com\r\n\r\n",
        Body::None,
        "example.com",
        80,
    );
    let curl = to_curl(&req, Shell::Posix);
    assert!(!curl.contains("--data-binary"), "{curl}");
    assert!(!curl.contains("-d"), "{curl}");
}

#[test]
fn curl_post_uses_data_binary() {
    let body = Bytes::from_static(b"name=value");
    let req = request(
        b"POST /submit HTTP/1.1\r\nHost: example.com\r\nContent-Length: 10\r\n\r\n",
        Body::inline(body),
        "example.com",
        80,
    );
    let curl = to_curl(&req, Shell::Posix);
    assert!(
        curl.contains("--data-binary 'name=value'"),
        "body must render via --data-binary: {curl}"
    );
    // Every header is a `-H` with a quoted value.
    assert!(curl.contains("-H 'Host: example.com'"), "{curl}");
    assert!(curl.contains("-H 'Content-Length: 10'"), "{curl}");
    assert!(curl.contains("-X 'POST'"), "{curl}");
}

#[test]
fn curl_blob_body_is_omitted_not_leaked() {
    let blob = Body::Blob {
        hash: [7u8; 32],
        len: 4096,
        truncated_at: None,
        capture_dropped: false,
        relay: None,
    };
    let req = request(
        b"POST /upload HTTP/1.1\r\nHost: example.com\r\n\r\n",
        blob,
        "example.com",
        80,
    );
    assert!(body_is_blob(&req));
    let curl = to_curl(&req, Shell::Posix);
    assert!(curl.contains("# body omitted"), "{curl}");
    assert!(!curl.contains("--data-binary"), "{curl}");
}

#[test]
fn to_raw_head_plus_inline_body() {
    let head = b"POST /p HTTP/1.1\r\nHost: example.com\r\nContent-Length: 5\r\n\r\n";
    let body = Bytes::from_static(b"hello");
    let with_body = request(head, Body::inline(body), "example.com", 80);
    let mut expected = head.to_vec();
    expected.extend_from_slice(b"hello");
    assert_eq!(to_raw(&with_body), expected);

    let no_body_head = b"GET /p HTTP/1.1\r\nHost: example.com\r\n\r\n";
    let no_body = request(no_body_head, Body::None, "example.com", 80);
    assert_eq!(to_raw(&no_body), no_body_head.to_vec());
}

#[test]
fn redacted_render_has_no_secret() {
    let jwt =
        "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTYifQ.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJ0adQssw5c";
    let head = format!(
        "POST /token HTTP/1.1\r\nHost: api.example.com\r\nAuthorization: Bearer {jwt}\r\nContent-Length: 20\r\n\r\n"
    );
    let req = tls_request(
        head.as_bytes(),
        Body::inline(Bytes::from_static(b"access_token=SENTINEL")),
        "api.example.com",
        443,
    );

    // The unredacted renders contain both secrets.
    let curl_clear = to_curl(&req, Shell::Posix);
    let raw_clear = to_raw(&req);
    assert!(curl_clear.contains(jwt));
    assert!(curl_clear.contains("SENTINEL"));
    assert!(contains(&raw_clear, jwt.as_bytes()));
    assert!(contains(&raw_clear, b"SENTINEL"));

    // The redacted renders contain neither.
    let policy = RedactionPolicy::default_policy();
    let redacted = policy.redact_message(&req);
    let curl_red = to_curl(&redacted, Shell::Posix);
    let raw_red = to_raw(&redacted);
    assert!(!curl_red.contains(jwt), "{curl_red}");
    assert!(!curl_red.contains("SENTINEL"), "{curl_red}");
    assert!(!contains(&raw_red, jwt.as_bytes()));
    assert!(!contains(&raw_red, b"SENTINEL"));
}

/// Whether `hay` contains `needle` as a contiguous byte run.
fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}
