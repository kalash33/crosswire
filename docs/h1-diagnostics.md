# Diagnostic catalogue

The 26 `DiagnosticCode` values of `crosswire-core`, as emitted by `crosswire-h1`.
`code` is the stable `as_str()` name. Severities are the defaults from
`DiagnosticCode::default_severity` for `Profile::Lenient` and `Profile::Strict`.
Neither profile makes any of these fatal (except `invalid_chunk_size`, which is a
`ParseError`); Strict only raises the severity.

This table is checked against the code by
`crates/crosswire-h1/tests/docs_catalogue.rs`.

| code | meaning | Lenient severity | Strict severity |
| --- | --- | --- | --- |
| `duplicate_content_length` | More than one Content-Length value, from repeated headers or a comma-separated list. The span is the first Content-Length value; if the values differ the first one frames the body. | Warning | Error |
| `conflicting_framing` | Transfer-Encoding and Content-Length are both present. The span is the first Transfer-Encoding value; Transfer-Encoding wins. | Warning | Error |
| `obs_fold` | A header line begins with whitespace and is folded into the previous header. The span is the continuation line. | Warning | Error |
| `bare_lf` | A line ends in LF without CR: a head line, a chunk-size line, the line break after chunk data, or a trailer line. The span is the terminator. | Warning | Error |
| `bare_cr` | A lone CR inside the content of a head line. The span is the CR byte. | Warning | Error |
| `whitespace_before_colon` | Whitespace between a header name and its colon. The span is the whitespace. | Warning | Error |
| `trailing_space_in_name` | The header name span ends in whitespace; always reported together with whitespace_before_colon. The span is the name. | Warning | Error |
| `leading_whitespace_before_start_line` | Whitespace or blank lines precede the start line and are skipped. The span is the skipped prefix. | Info | Error |
| `non_canonical_header_case` | A valid header name is not in Title-Case, for example `host` or `X-FOO`. The span is the name. | Info | Info |
| `invalid_token_char_in_name` | A header name is empty or has a byte that is not an HTTP token character, or the line has no colon. The span is the name with trailing whitespace trimmed, or the whole line when there is no colon. | Warning | Error |
| `invalid_char_in_value` | A header value contains a control byte other than HTAB and CR (0x00-0x1F, 0x7F). Reported at most once on the header's first line and at most once on each obs-fold continuation line, on the first such byte. | Warning | Error |
| `missing_host` | A request has no Host header. The span is empty and sits at the end of the head. | Warning | Error |
| `duplicate_host` | A request has a second or later Host header. The span is that header's name. | Warning | Error |
| `absolute_form_target` | The request target starts with `scheme://`. The span is the target. | Info | Info |
| `chunk_extension_present` | A chunk-size line has an extension after `;`. The span is the extension text. Also delivered as `Progress::ChunkExt`. | Info | Info |
| `trailer_present` | A chunked body ends with at least one trailer field. The span is the first trailer line. Also delivered as `Progress::Trailers`. | Info | Info |
| `chunk_size_leading_zeros` | A chunk size has more than one digit and starts with `0`. The span is the size token. | Info | Warning |
| `invalid_chunk_size` | A chunk size is not hexadecimal, or the line break after chunk data is malformed. Reported as the fatal `ParseError::InvalidChunkSize`, never as an entry in a diagnostics list. | Error | Error |
| `content_length_non_numeric` | A Content-Length value is not ASCII digits, is empty, or overflows `u64`. The span is that value. Unless Transfer-Encoding is present, the framing becomes `Ambiguous` with rule `InvalidContentLength` (no body for requests, until-close for responses), even when another value is valid. | Warning | Error |
| `content_length_leading_plus` | A Content-Length value starts with `+`; the sign is tolerated. Reported once per message, on the first such value. | Warning | Error |
| `unknown_transfer_coding` | The last Transfer-Encoding coding is not `chunked`. Framing falls back to until-close for responses and to no body for requests. | Warning | Error |
| `body_after_no_body_status` | A response whose status forbids Content-Length and Transfer-Encoding carries one: 1xx other than 101 and 204 (framing `Ambiguous`, resolved to no body, rule `NoBodyStatus`), or 101 and a 2xx answer to CONNECT (resolved to `Upgrade`, rule `ConnectSuccess`). Not reported for a response to HEAD or a 304, where these headers are allowed (RFC 9110 sections 8.6, 9.3.2) and the framing is plain no body. The span is the first such value. | Warning | Error |
| `expect_100_continue` | An `Expect: 100-continue` header is present. The span is the value. | Info | Info |
| `http_0_9_request` | The request line has fewer than two spaces, so there is no version. The span is the whole line. | Warning | Error |
| `version_unusual` | The version is not exactly `HTTP/1.0` or `HTTP/1.1`. For responses it is also the catch-all when the status is not three ASCII digits. At most once per response; the span is the version. | Warning | Error |
| `chunk_size_whitespace` | Spaces or tabs between the chunk size and the `;` of an extension or the line break (`5 ;x=y`, `5 \r\n`). RFC 9112 allows this BWS before `;`; it is tolerated, never fatal. Reported once per chunk; the span is the whitespace. Whitespace before or inside the size token is still `invalid_chunk_size`. | Info | Warning |
