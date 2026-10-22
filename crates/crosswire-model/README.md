# crosswire-model

The exchange schema and the sink and event types that connect the Crosswire
proxy, store and API. Part of
[Crosswire](https://github.com/kalash33/crosswire/blob/main/README.md), an open-source Rust web security testing platform.

An `Exchange` is one request and its response as recorded: the head bytes exactly
as they were on the wire, the spans and framing the parser found, diagnostics,
the body, any recorded edits, and the connection facts (target, SNI, TLS
version, chain result). This crate is the single definition of that shape. The
store writes it, the API will serve it, the proxy will fill it.

## What it does and does not do

- Defines the serde schema (`Exchange`, `Message`, `Head`, `Body`, `Marker`,
  `Event`) and the `ExchangeSink` trait the proxy hands finished exchanges to.
- Builds a `Message` from a parsed HTTP/1 head (`Message::http1_raw`), copying
  the head bytes and the `crosswire-h1` spans, so a recorded message can be
  written back byte for byte.
- Applies a `RedactionPolicy` to a message. Redaction rebuilds the head by span
  and re-parses it, and labels the result `Fidelity::Normalized`.
- Does no I/O, no networking and no storage. It never reads the clock except to
  mint a ULID in `Exchange::new` and `Marker::new` (`std` only).
- Does not redact inline bodies yet; only the head, the request target and the
  recorded diffs are redacted. Blob bodies are untouched.

## Key types

| type | what it is |
| --- | --- |
| `Exchange` | request, optional response, `Timing`, `ScopeState`, tags, notes, `Namespace`, `ExchangeState`; keyed by a ULID |
| `Message` | one request or response: `MessageKind`, `Fidelity`, `Head`, `FramingRecord`, diagnostics, `Body`, `Diff`s, `ConnectionInfo` |
| `Head` | `Http1Raw` (bytes plus `HeadSpans`), `Normalized` (header pairs) or `Tunnel` (counters); `#[non_exhaustive]` |
| `Body` | `None`, `Inline` (raw framed bytes) or `Blob` (SHA-256, length, `truncated_at`, `capture_dropped`) |
| `ConnectionInfo`, `ChainResult` | target address and port, SNI, ALPN, TLS version and cipher, upstream chain result (`NotTls`, `Valid`, `Invalid`, `Overridden`), connection id and reuse flag |
| `Marker`, `MarkerKind` | gap, limit-exceeded and error rows; a `Gap` carries a ULID time window |
| `Event` | `Received`, `Committed`, `Marker`, `Error`, `LimitExceeded`, `Lagged`; every variant has a monotonic `seq` |
| `ExchangeSink`, `SinkError` | `submit(&self, Exchange) -> Result<(), SinkError>`; `Full` or `Closed` |
| `RedactionPolicy` | header names redacted whole plus value patterns (`Bearer `, `access_token=`, `api_key=`, JWT shape); `default_policy()` |
| `SCHEMA_VERSION` | `1` |

## Defaults and constants

| name | value | where |
| --- | --- | --- |
| `SCHEMA_VERSION` | 1 | `lib.rs` |
| `HTTP1_PARSER_VERSION` | 1 (recorded beside every `Http1Raw` head, so spans can be re-derived if the parser changes) | `message.rs` |
| `RedactionPolicy::JWT_PATTERN` | `"@jwt"` | `redact.rs` |

## Schema rules

- Additive-only from the 0.1.0 tag: fields and variants may be added, never
  removed or renamed, and readers ignore unknown fields (no type uses
  `deny_unknown_fields`). Until that tag the schema is at version 1 but has not
  been released; a deliberate break would bump `SCHEMA_VERSION`.
- Every enum is `#[non_exhaustive]` with kebab-case wire names. The plain
  string enums (`Fidelity`, `MessageKind`, `MarkerKind`, `TunnelReason`) read an
  unrecognised name as `Unknown`. The tagged enums (`Head`, `Body`,
  `ChainResult`, `Event`) fail to deserialise a variant they do not know, so a
  reader must be upgraded before data that uses a new variant reaches it.
- Byte payloads are base64 strings under the default `std` feature and byte
  sequences without it. Both ends of a stored document must agree on the
  feature; project files and the API use `std`.

## Example

```rust,ignore
use crosswire_h1::{Parser, ParserConfig, Progress};
use crosswire_model::{Body, ConnectionInfo, Exchange, Message, RedactionPolicy};

let wire = b"GET /a HTTP/1.1\r\nHost: example.com\r\nAuthorization: Bearer abc\r\n\r\n";
let mut parser = Parser::request(&ParserConfig::default());
let step = parser.feed(wire).expect("fatal parse error");
let Progress::Head(head) = step.event else { panic!("incomplete head") };

let request = Message::http1_raw(&head, Body::None, ConnectionInfo::new("93.184.216.34", 443));
let exchange = Exchange::new(request);

// A copy that is safe to share: Authorization becomes [REDACTED].
let shareable = RedactionPolicy::default_policy().redact_message(exchange.request());
```

The recipe that records a live exchange (send, read, build, submit) is
`record_exchange` in `crates/crosswire-recorder-tests/src/lib.rs`.

## Features

| feature | default | effect |
| --- | --- | --- |
| `std` | yes | enables `std` in dependencies, base64 byte payloads, `Exchange::new`, `Marker::new`, `std::error::Error` for `SinkError`; the crate is `no_std` + `alloc` without it (use `Exchange::with_id` and `Marker::with_id`) |

## Status

Pre-alpha, `publish = false`. Only the schema is held additive-only (at schema
version 1, from the 0.1.0 tag); the Rust API around it can still change.
`crosswire-core` and `crosswire-h1` are path dependencies that are not
published, so `cargo package -p crosswire-model` fails (`no matching package
named crosswire-core found`) until they are.

Design: [spec section 6.1](https://github.com/kalash33/crosswire/blob/main/docs/superpowers/specs/2026-10-22-crosswire-design.md)
(the schema) and section 11.4 (redaction). On-disk use of the schema is in
[docs/format.md](https://github.com/kalash33/crosswire/blob/main/docs/format.md).
