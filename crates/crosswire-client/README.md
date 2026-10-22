# crosswire-client

A recording upstream HTTP/1.1 client: the code a Crosswire proxy, repeater or
test uses to talk to a server. Part of
[Crosswire](https://github.com/kalash33/crosswire/blob/main/README.md), an open-source Rust web security testing platform.

It resolves a name, checks the resolved addresses against a destination gate,
connects to those same addresses, runs TLS if asked, writes the exact bytes you
give it, and reads the response with
[`crosswire-h1`](https://github.com/kalash33/crosswire/blob/main/crates/crosswire-h1/README.md),
keeping the bytes it consumed so the exchange can be recorded as it crossed the
wire.

## What it does and does not do

- `Transport::connect` does DNS (5 s, 64 lookups at a time), the destination
  gate, TCP connect, and, for TLS targets, the handshake through a `TlsBackend`.
- `Session::send` writes a `WritePlan` verbatim and returns a `ResponseStream`
  that yields the head and body pieces incrementally. It never rewrites the
  request: no added headers, no header normalisation.
- Keeps `raw_response()`, every response byte the parser consumed (interim
  responses, chunk framing and trailers included), up to a cap. Body pieces keep
  streaming past the cap.
- Has **no proxy support**: no upstream HTTP or SOCKS proxy, no CONNECT.
- Has **no HTTP/2** (ALPN offers only `http/1.1` by default) and no redirects,
  cookies, retries, compression handling or connection pool. One `Session` is
  one connection, one exchange at a time.
- Does not run hold points. `HoldPoint` and `Segment::hold` are reserved, and
  `send` refuses a plan that uses one (`ClientError::Protocol`).
- `Expect: 100-continue` is not honoured: the whole plan is written before any
  response byte is read.
- Does not record anything itself. The recipe that turns a session into an
  `Exchange` and submits it is `record_exchange` in
  [`crosswire-recorder-tests`](https://github.com/kalash33/crosswire/blob/main/crates/crosswire-recorder-tests/src/lib.rs).

## Key types

| type | what it is |
| --- | --- |
| `Transport` | `new(Arc<dyn TlsBackend>, Arc<DestinationGate>)`; `connect(&Target) -> Connection` |
| `Target` | `plain(host, port)`, `tls(host, port, sni, VerifyMode)`, `with_tls_config` |
| `DestinationGate`, `GateRefusal` | the destination policy and why it refused (`Loopback`, `LinkLocal`, `Metadata`, `OwnListener`) |
| `Connection` | the stream plus `ConnectionInfo`, `connect_ms`, `tls_ms`, `chain_error` |
| `TlsBackend`, `RustlsBackend` | the pluggable TLS client trait and its rustls implementation (TLS 1.2 and up, ring provider) |
| `TlsConnectConfig`, `VerifyMode`, `TlsInfo` | handshake parameters (SNI, ALPN, client certificate), `WebPki` or `AcceptInvalid`, and what was negotiated |
| `Session` | `new(Connection, ParserConfig)`, `send`, `is_reusable`, `info`, `connection`, builders for the timeout and caps |
| `ResponseStream` | `head`, `next_body`, `request_bytes`, `raw_response`, `raw_len`, `raw_truncated_at`, `ttfb_ms`, `interim_responses`, `body_diagnostics` |
| `BodyPiece` | `raw` (as on the wire), `data` (decoded), `ext`, `trailers` |
| `WritePlan`, `Segment`, `HoldPoint` | the exact bytes of a request, in order |
| `ClientError` | `Dns`, `Refused`, `Io`, `Tls`, `Timeout`, `Protocol`; `#[non_exhaustive]` |

## Limits and defaults

| limit | default | constant or setter |
| --- | --- | --- |
| DNS timeout | 5 s | `DNS_TIMEOUT` (`transport.rs`) |
| Concurrent DNS lookups | 64 | `DNS_CONCURRENCY` |
| TCP connect timeout, per resolved address | 10 s | `TCP_CONNECT_TIMEOUT` |
| TLS handshake timeout | 15 s | `HANDSHAKE_TIMEOUT` (`rustls_backend.rs`) |
| Read or write idle timeout | 60 s | `DEFAULT_READ_TIMEOUT`, `Session::with_read_timeout` |
| Retained response bytes (`raw_response`) | 50 MiB | `DEFAULT_RAW_CAP`, `Session::with_raw_cap` |
| Interim (1xx) responses per exchange | 16 | `DEFAULT_MAX_INTERIM`, `Session::with_max_interim` |
| Socket read size | 16 KiB | `READ_CHUNK` (`stream.rs`) |
| Response head and line limits | those of the `ParserConfig` you pass; h1's defaults are 64 KiB head, 256 headers, 16 KiB line | `crosswire-h1` |

The 50 MiB raw cap bounds what the stream keeps for recording, not what it reads.
When it is hit, `raw_truncated_at()` is set and `raw_response()` stops growing.
`record_exchange` treats that as an error and records nothing, rather than store a
partial response as if it were whole.

## Security notes

- **Destination gate.** `DestinationGate::check` runs on the resolved
  addresses, so the spelling of the host (`localhost`, `127.1`, `2130706433`,
  `::ffff:127.0.0.1`, a name that resolves to loopback) does not matter. It
  refuses loopback and unspecified addresses, link-local ranges
  (`169.254.0.0/16`, `fe80::/10`), and the metadata addresses `169.254.169.254`,
  `100.100.100.200` and `fd00:ec2::254`. IPv4-mapped IPv6 addresses are treated
  as IPv4. Every resolved address must pass, and an empty result is refused.
  Private ranges such as `10.0.0.0/8` are not blocked: testers need them.
- **Allowlist.** `allow(host, port)` is an exact pair, compared without case or a
  trailing dot, and it lifts the loopback, link-local and metadata checks for
  that name, whatever it resolves to. The own-listener ports passed to
  `DestinationGate::new` are refused on loopback even when allowlisted.
- **No second lookup.** `connect` dials the addresses the gate checked; it does
  not resolve the name again, so DNS rebinding between check and connect does not
  apply.
- **`VerifyMode::WebPki`** verifies the chain against the bundled Mozilla roots
  (`webpki-roots`) and the SNI name, never a Host header, and fails the
  handshake on a bad chain. `RustlsBackend::with_extra_roots` adds DER trust
  anchors.
- **`VerifyMode::AcceptInvalid` is record-and-allow, not silent.** It runs the
  same verification, and if it fails the handshake continues with the reason
  kept: `TlsInfo::chain_result()` is `Overridden`, `chain_error()` and
  `Connection::chain_error()` carry the reason, and `ConnectionInfo` records
  `cert_override` as true so the exchange shows the override. Handshake
  signatures are still checked. It is meant for a per-host override and is never
  a default; the policy that applies it belongs to the caller.
- A bad SNI is an error (`ClientError::Tls`), never a fallback to another name.
- `TlsConnectConfig`'s `Debug` output does not print the client key.

## Example

```rust,ignore
use std::sync::Arc;
use bytes::Bytes;
use crosswire_client::{
    DestinationGate, RustlsBackend, Session, Target, Transport, VerifyMode, WritePlan,
};
use crosswire_h1::{ParserConfig, ResponseContext};

let transport = Transport::new(
    Arc::new(RustlsBackend::webpki_roots()),
    Arc::new(DestinationGate::new(vec![8080])), // our own listener port
);
let target = Target::tls("example.com", 443, "example.com", VerifyMode::WebPki);
let conn = transport.connect(&target).await?;

let mut session = Session::new(conn, ParserConfig::default());
let plan = WritePlan::raw(Bytes::from_static(
    b"GET / HTTP/1.1\r\nHost: example.com\r\nConnection: close\r\n\r\n",
));
let mut stream = session.send(plan, ResponseContext::for_method(b"GET")).await?;
let head = stream.head().await?; // crosswire_h1::RawMessage
while let Some(piece) = stream.next_body().await? {
    // piece.data is the decoded payload, piece.raw the bytes as on the wire
}
let wire = stream.raw_response(); // head and framed body, byte for byte
```

## Status

Pre-alpha, `publish = false`; the API is not frozen. It depends on the unpublished
`crosswire-core`, `crosswire-h1` and `crosswire-model`, so it cannot be packaged yet.

Design: [spec sections 5.2 (types), 11.2 (destination gate) and 11.9 (upstream TLS policy)](https://github.com/kalash33/crosswire/blob/main/docs/superpowers/specs/2026-10-22-crosswire-design.md).
