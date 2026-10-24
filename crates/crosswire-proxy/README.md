# crosswire-proxy

The HTTP/1.1 intercepting proxy engine: it terminates TLS with leaves from the
Crosswire CA, relays every message byte for byte, and records what crossed into
an `ExchangeSink`. Part of
[Crosswire](https://github.com/kalash33/crosswire/blob/main/README.md), an open-source Rust web security testing platform.

A `Proxy` listens for a browser's CONNECT and absolute-form requests, checks the
destination gate in `crosswire-client`, connects upstream, and pumps bytes in
both directions while two `crosswire-h1` parsers watch where the messages begin
and end. It never rebuilds a message from a model: what the parser consumed is
what is forwarded, and a bounded copy of it is what is stored.

The design and its alternatives are in
[ADR-0003](https://github.com/kalash33/crosswire/blob/main/docs/adr/0003-proxy-relay.md).
The `crosswire` binary that wires this crate to the store is
[`crosswire-server`](https://github.com/kalash33/crosswire/blob/main/crates/crosswire-server/README.md).

## What it does and does not do

- Accepts CONNECT and absolute-form requests (`GET http://host/...`). An
  origin-form request is refused with a generated 400, except one whose `Host`
  is the proxy itself, which gets the info page (below).
- For CONNECT it answers `200 Connection established`, peeks the TLS
  ClientHello for the SNI, and either tunnels the bytes untouched (passthrough)
  or terminates TLS with a leaf for the SNI (or the CONNECT authority when
  there is none) and relays the decrypted HTTP/1.1. A client that sends plain
  HTTP inside the tunnel is relayed as plain HTTP.
- Relays as a **byte pump**: forwards exactly the bytes the parsers consumed,
  with write-before-read backpressure and a 64 KiB read buffer. The request body
  is relayed while the upstream is being read, so `Expect: 100-continue` and
  early responses (a 401 or 413 during an upload) work. Keep-alive and
  pipelining are supported; the relay answers one request at a time per
  connection.
- Changes only the request head, and records each change as a diff: it deletes
  `Proxy-Connection` and `Proxy-Authorization` lines, and on the plain path it
  turns an absolute-form target into origin-form and inserts a `Host` header when
  the request has none. Nothing else is touched, including header case and order,
  `Connection`, and body framing.
- Records every relayed exchange with `Fidelity::Raw` and the diagnostics the parsers
  found, interim (1xx) heads, the peer address, timings, and bodies. Stored
  bodies are the **raw framed wire bytes**; `relay` (length and SHA-256 of
  everything that crossed) is recorded only when the stored bytes are not the
  whole body (cut at `capture_cap`, dropped for the budget, or cut short).
- Treats a message it cannot delimit (including a request with an invalid
  `Content-Length`), a 101 upgrade, and a 2xx answer to a `CONNECT` sent
  inside a tunnel as the end of HTTP: the connection becomes an opaque tunnel
  and a `Tunnel` exchange records the byte counts (reason `Undelimitable` or
  `Ws`).
- Calls a `Handler` at the four message points. `Decision` has `Forward` and
  `Drop` only; `Hold` and `Modify` arrive with intercept in alpha.2.
- Does **not** use `crosswire_client::Session` on the proxy path (it buffers
  whole requests); it takes the gate, DNS, TCP and TLS from `Transport` only.
- Does **not** do HTTP/2 (the leaf's ALPN offers only `http/1.1`, so browsers
  fall back), HTTP/3, proxy chaining, upstream proxies or SOCKS.
- Does **not** do automatic passthrough after handshake rejections. That needs
  scope evaluation (alpha.2), so in alpha.1 rejections are counted and one
  `Error` marker is written per window; the host stays intercepted. Manual
  passthrough works.
- Does **not** stream bodies to a pack-writer; bodies go inline through the
  store's queue (ADR-0003).
- Does **not** install or trust the CA anywhere. See
  [docs/ca-install.md](https://github.com/kalash33/crosswire/blob/main/docs/ca-install.md).
- Redacts nothing. It records what it received; redaction is applied on the way
  out (exports, logs, copy-as), spec section 11.4.

## Key types

| type | what it is |
| --- | --- |
| `Proxy` | `bind(cfg, deps)`, `from_listener(cfg, TcpListener, deps)`, `local_addr`, `counters`, `run(watch::Receiver<bool>)` |
| `ProxyConfig`, `ProxyAuth` | the bind address, `allow_non_loopback`, optional Basic credentials, `passthrough` and `accept_invalid_cert` host lists, and the limits; `validate()` |
| `ProxyDeps` | the leaf issuer, the CA as DER, PEM and SPKI pin (served by the info page), the `Transport`, the `ExchangeSink`, the `Handler` |
| `ProxyLimits`, `ProxyLimitsBuilder` | every limit below, built from the spec defaults |
| `Handler`, `Decision`, `NoopHandler` | `on_request_head`, `on_request_body_chunk`, `on_response_head`, `on_response_body_chunk`; `Forward` or `Drop`; `#[non_exhaustive]` |
| `Counters` | atomic counts: `accepted`, `refused_max_connections`, `refused_per_ip`, `refused_pending_connect`, `auth_failures`, `handshake_rejections`, `tunnels`, `exchanges`, `sink_full` |
| `ProxyError` | `Config`, `NonLoopbackBind`, `AuthRequired`, `Bind`, `Io`, `Client`, `Tls`, `Protocol`, `Timeout`, `Limit`; `#[non_exhaustive]` |
| `parse_sni`, `SniError` | read the `server_name` from the first TLS record without consuming it |

## Listener shape

- **Loopback by default.** `ProxyConfig::validate` refuses a non-loopback
  `bind` unless `allow_non_loopback` is set, and then also refuses it without
  `proxy_auth`.
- **Proxy authentication** is `Proxy-Authorization: Basic`, compared in constant
  time, and applies to CONNECT and absolute-form requests. A failure gets a 407
  with `Proxy-Authenticate: Basic realm="crosswire"` and is counted in
  `auth_failures`. Credentials are checked on the first request of a
  connection and are not re-checked for later requests on it. `Debug` for `ProxyAuth` redacts the password, and the
  `Proxy-Authorization` line is removed before a request goes upstream.
- **The info page** is served for a `GET` whose `Host` is `crosswire` (with or
  without a port), the listener's own `host:port`, or `localhost:PORT` on a
  loopback listener, and for absolute-form `http://crosswire/...`. It is not
  behind proxy authentication. Routes: `/` (HTML), `/ca.der`
  (`application/pkix-cert`), `/ca.pem` (`application/x-pem-file`), `/ca.spki`
  (the base64 SHA-256 pin, plain text); any other path, and any method other than
  `GET`, is a 404. Every response carries `Connection: close`,
  `X-Content-Type-Options: nosniff`,
  `Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'` and
  `Referrer-Policy: no-referrer`. The page has no scripts, event handlers or
  absolute URLs and points at `docs/ca-install.md`.
- **Targets** are validated: a host of at most 253 bytes with labels of 1 to 63
  of `[A-Za-z0-9-]`, an IPv4 literal, or a bracketed IPv6 literal; a port of 1 to
  5 digits and nonzero. Anything else is a generated 400.
- **Passthrough** rules are an exact host or a `.suffix` (which does not match
  the apex), case-insensitive. A CONNECT is tunnelled when the SNI **or** the
  CONNECT authority matches.

## Failure behaviour

A failure before the response head becomes a generated response, recorded as an
`Error` exchange with `Fidelity::Generated`; after the head the client has been
told something, so the relay closes or tunnels and records what happened.
Substituted text in a generated body or marker is capped at 1,024 bytes.

| failure | the client sees | recorded | marker |
| --- | --- | --- | --- |
| request head times out, or the client closes or fails mid-head | close | nothing | none |
| request head does not parse, or is over a limit, inside a tunnel or on a later request | 400 `Crosswire could not parse the request: {code}` | nothing | `LimitExceeded` for the limit errors, else `Error` |
| the first request on a connection does not parse, is not a proxy request, or has a bad target | 400 (431 for an oversize head, 407 for missing credentials) with a fixed text | nothing | none |
| request body does not parse | 400 | `Error`, request as captured | as above |
| request has an invalid `Content-Length` (its body cannot be delimited) | the head is sent upstream, then an opaque tunnel carries the rest | `Error` (request only), then `Tunnel` (`Undelimitable`) | `Error`: `undelimitable request to {host}:{port}: invalid Content-Length` |
| upstream connect: DNS, I/O, protocol | 502 `Crosswire could not connect to {host}:{port}: {reason}` | `Error`, generated response | none |
| destination refused by the gate (including the proxy's own port) | 502 with the `--allow host:port` hint | `Error`, generated | none |
| upstream certificate refused | 502 with the `--accept-invalid-cert host` hint | `Error`, generated | none |
| upstream connect timeout | 504 phase `connect` | `Error`, generated | none |
| upstream closes before any response byte | retry once (rule below), else 502 | `Error`, generated | none |
| upstream closes in the middle of the head, or fails to read | 502 | `Error`, generated | none |
| more than 16 interim responses | 502 | `Error`, generated | none |
| no final head within `idle` after the body, or past `total` | 504 phase `response head` | `Error`, generated | none |
| response head or status does not parse | the raw bytes, then an opaque tunnel | `Error` (partial), then `Tunnel` (`Undelimitable`) | `Error`: `undelimitable response from {host}:{port}: {code}` |
| response body does not parse (a bad chunk) | the bytes so far, then an opaque tunnel | as above | as above |
| upstream closes or fails mid-body | the bytes so far, then close | `Error`, body cut | `Error`: `upstream {host}:{port} closed mid-body: ...` or `failed mid-body: ...` |
| `idle` or `total` timeout during a body | close | `Error`, body cut | `Error`: `timeout: {phase} {host}:{port}` |
| client leaves mid-request-body or mid-response | none | `Error`, body cut | none (debug log) |
| a final response arrives while the request body is still uploading | the response, then close | as normal, request body cut | none |
| 101 Switching Protocols | the head, then an opaque tunnel | `Received` (framing `Upgrade`), then `Tunnel` (`Ws`) | none |
| 2xx to a `CONNECT` sent inside a tunnel | the head, then an opaque tunnel | `Received` (framing `Upgrade`), then `Tunnel` (`Undelimitable`) | none |
| extra bytes after a complete response | the response only; the upstream connection is dropped | the exchange | `Error`: `extra bytes after response from {host}:{port}` |
| request pipeline deeper than `max_pipelined` | the response to the request in progress, then close | that exchange | `LimitExceeded`: `pipelining: {n} requests queued on one connection from {peer} (max {m})` |
| rewritten request head does not re-parse | unaffected | the original head | `Error`: `rewritten request head did not re-parse` |
| after a CONNECT 200: passthrough upstream connect fails | close | nothing | `Error`: `passthrough upstream failed for {host}:{port}: {reason}` |
| after a CONNECT 200: no leaf can be issued | close | nothing | `Error`: `leaf issue failed for {host}: {reason}` |
| after a CONNECT 200: client aborts or stalls the TLS handshake | close | nothing | counted; `Error` once per window (below) |

In the table, `{code}` is the message of the `crosswire-h1` `ParseError`, for
example `message head too large`, `too many headers`, `line too long`, `too many
chunks` or `invalid chunk size at bytes 0..2`.

**Retry rule.** A request is retried once on a fresh connection when it was on a
reused keep-alive connection that the upstream closed (a FIN, or a reset such as
`ConnectionReset` or `BrokenPipe`) before sending any response byte, the request
has no body (no framing, or `Content-Length: 0`), no interim response was seen,
and the method is GET, HEAD, OPTIONS or TRACE.

**Handshake rejections.** A client that aborts or stalls the handshake with a
Crosswire leaf is almost always one that does not trust the CA. Each is counted
in `handshake_rejections` and per host; when `rejections_per_window` land in
`rejection_window`, one `Error` marker is written, then at most one more per
window: `Browser rejected the Crosswire certificate for {host} — is the CA
trusted?`. The host is **not** switched to passthrough.

## Limits and defaults

Defaults are `ProxyLimits::default()` (spec section 11.6); each field has a
builder setter of the same name.

| limit | default | field or constant |
| --- | --- | --- |
| Body bytes kept per body | 50 MiB; longer sets `truncated_at` | `capture_cap` |
| In-flight capture budget, all connections | 256 MiB; a refusal sets `capture_dropped` | `body_budget` |
| Capture buffer growth | 4 KiB first, then doubling, never past `capture_cap` | `FIRST_STEP` (`capture.rs`) |
| Read buffer per direction | 64 KiB | `READ_BUF` (`pump.rs`), `BUF` (`tunnel.rs`) |
| Request head read, first head on a connection | 30 s | `header_read` |
| Idle, per read or write, and between requests | 60 s | `idle` |
| One exchange, from the first request byte | 300 s | `total` |
| Client TLS handshake | 15 s | `tls_handshake` |
| Wait for the ClientHello after the 200 | 5 s | `SNI_PEEK_TIMEOUT` (`mitm.rs`) |
| Idle on an opaque tunnel | off | `tunnel_idle` |
| Simultaneous connections | 4,000 | `max_connections` |
| CONNECTs awaiting tunnel establishment | 256 | `max_pending_connect` |
| Connections per non-loopback client IP | 64; loopback is exempt | `max_per_client_ip` |
| Pipelined requests queued | 16 | `max_pipelined` |
| Interim responses per exchange | 16 | `MAX_INTERIM` (`relay.rs`) |
| Handshake rejections that raise a marker | 3 per 30 s | `rejections_per_window`, `rejection_window` |
| Hosts tracked for rejections | 1,024, least recently rejected forgotten first | `MAX_HOSTS` (`rejections.rs`) |
| SNI and target host | at most 253 bytes, valid hostname | `MAX_NAME` (`sni.rs`), `valid_authority` (`front.rs`) |
| Parser limits | head 64 KiB, 256 headers, line 16 KiB, chunk-size line 4 KiB, trailers 16 KiB, 1,000,000 chunks, 64 body diagnostics | `parse` (`crosswire_h1::ParserConfig`) |
| Substituted text in generated bodies and markers | 1,024 bytes | `MAX_SUBSTITUTION` (`generated.rs`) |
| Drain after a refusal or an early response | 2 s and 256 KiB | `LINGER`, `LINGER_BYTES` (`listener.rs`) |
| Wait for live connections at shutdown | 5 s, then abort | `SHUTDOWN_GRACE` (`listener.rs`) |
| Warning for a refused submit | once per 10 s | `REFUSAL_WARN_EVERY` (`record.rs`) |
| `inline_memory` | 1 MiB; **not consulted yet** | `inline_memory` |

The leaf issuer's limits (4,096 cached leaves, 50 mints per second) are in
[`crosswire-ca`](https://github.com/kalash33/crosswire/blob/main/crates/crosswire-ca/README.md); the store's queue (64 MiB, 10,000 exchanges) and journal (1 GiB) are in
[`crosswire-store`](https://github.com/kalash33/crosswire/blob/main/crates/crosswire-store/README.md).

### Memory

Capture memory is at most `body_budget` by construction: every byte of buffer
capacity is reserved from the budget before it is allocated, and the reservation
is held until the sink has taken or refused the exchange. Three qualifications:

- Utilisation can be low. A 100-byte body holds a 4 KiB buffer, and a buffer that
  has just doubled is half empty.
- During a growth the old buffer is briefly alive beside the new one and is not
  charged. At the default cap the largest growth is from 32 MiB to 50 MiB, so up
  to 32 MiB per capture that is growing at that moment is outside the budget.
- After `submit`, the bytes leave the proxy budget and only the store's queue
  bounds them.

The relay itself holds at most one 64 KiB read buffer per direction, so a
256 MiB until-close response with `capture_cap` of 1 MiB runs in flat memory
(`tests/limits.rs`, `rss_flat_under_large_until_close_response`).

## Security notes

- The proxy binds loopback unless told otherwise, and then demands credentials. A non-loopback proxy hands anyone who authenticates a
  way to reach whatever the machine can; the destination gate still refuses
  loopback, link-local and metadata ranges and the proxy's own port, evaluated
  after DNS, unless a host is allowed with `--allow`.
- Upstream certificates are verified against the web PKI. A host on
  `accept_invalid_cert` is accepted with the error recorded on the connection;
  the binary audits each such host.
- The CA key never leaves `crosswire-ca`. Leaves are issued for whatever name the
  client asked for (SNI or authority), which is why the CA must be trusted only
  on machines used for testing.
- Logs carry no message bytes: only addresses, hosts, counts and error text.
- The proxy records credentials, cookies and tokens exactly as they crossed. The
  project directory is as sensitive as the traffic; the store creates it 0700.
- Markers are memory-only until the store commits them: they are not spilled to
  the journal, so one raised just before a crash can be lost, and one raised
  while the store's queue is full is refused (`sink_full`).
- Stored bodies are the raw wire bytes, so a hostile server's body is stored as
  sent. Nothing in this crate decodes, decompresses or renders it.

## Example

```rust,ignore
use std::sync::Arc;
use crosswire_ca::{Ca, FileKeyStore, IssuerLimits, LeafIssuer};
use crosswire_client::{DestinationGate, RustlsBackend, Transport};
use crosswire_proxy::{NoopHandler, Proxy, ProxyConfig, ProxyDeps};
use tokio::net::TcpListener;
use tokio::sync::watch;

let ca = Arc::new(Ca::load_or_generate(&FileKeyStore::new(ca_dir), "Crosswire CA")?);

// Bind first: the gate must refuse the proxy's own port.
let listener = TcpListener::bind("127.0.0.1:8080").await?;
let gate = DestinationGate::new(vec![listener.local_addr()?.port()]);

let deps = ProxyDeps {
    issuer: Arc::new(LeafIssuer::new(Arc::clone(&ca), IssuerLimits::default())),
    ca_der: ca.cert_der(),
    ca_pem: ca.cert_pem(),
    ca_spki: ca.spki_sha256_base64(),
    transport: Arc::new(Transport::new(Arc::new(RustlsBackend::webpki_roots()), Arc::new(gate))),
    sink: store.sink(), // Arc<dyn ExchangeSink>, from crosswire-store
    handler: Arc::new(NoopHandler),
    resolve_override: None,
};
let cfg = ProxyConfig::new("127.0.0.1:8080".parse()?);
let proxy = Proxy::from_listener(cfg, listener, deps)?;

let (stop, stopped) = watch::channel(false);
let task = tokio::spawn(proxy.run(stopped));
// ... later: stop.send(true)?; task.await??;
```

## Tests

- `tests/corpus_relay.rs`: the `corpus/h1` cases through a CONNECT tunnel, whole
  and in 7-byte fragments: 104 cases, 208 runs, 190 byte-exact clean relays and
  18 error-path runs, with every one of the 26 diagnostic codes asserted
  covered.
- `tests/limits.rs`: 11 hostile-input fixtures, one per limit (head, headers,
  line, chunk line, chunk count, pending CONNECTs, per-IP cap, SNI length, tunnel
  idle, total timeout, flat memory under a 256 MiB response).
- `tests/failures.rs`: the failure table above.
- `tests/browser.rs`: a headless Chromium session recorded end to end. It is
  ignored by default; `just browser-test` runs it with a Chromium-based browser
  from `CROSSWIRE_BROWSER`, `google-chrome`, `chromium` or `chromium-browser`.
  CI runs it on the ubuntu job only.

## Status

Pre-alpha, `publish = false`; the API is not frozen. The crate depends on the
unpublished `crosswire-core`, `-h1`, `-model`, `-ca` and `-client`, so it cannot
be packaged yet.

Design: [spec section 7.1](https://github.com/kalash33/crosswire/blob/main/docs/superpowers/specs/2026-10-22-crosswire-design.md)
(proxying), section 11 (security of the tool: listeners, CA, `Limits`, upstream
TLS policy), and
[ADR-0003](https://github.com/kalash33/crosswire/blob/main/docs/adr/0003-proxy-relay.md).
