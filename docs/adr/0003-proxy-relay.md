# ADR-0003: the proxy relay is a byte pump with a bounded capture window

Status: Accepted

## Context

The proxy sits between a browser and a server and records what crossed. Two
promises from the spec pull on its design at the same time.

1. **Fidelity (spec section 6).** What the client sent is what the server
   receives, byte for byte, and what the server sent is what the client
   receives. Malformed, ambiguous and deliberately odd HTTP must pass through
   as it came, and be recorded as it came, with diagnostics instead of repairs.
2. **Never block on the store (spec sections 7.1 and 9.3).** A slow disk, a
   full queue or a refused submit must never stall a connection. Memory stays
   bounded however many bodies are in flight and however large they are.

A library that parses a message into a model and writes it back out breaks the
first promise whenever the model cannot hold what was on the wire. A relay that
keeps whole bodies until the exchange ends breaks the second the moment someone
downloads a large file. This ADR records how `crosswire-proxy` keeps both, what
it chose not to build, and the places where the memory bound is weaker than it
sounds.

## Decision

1. **Two parsers, one byte pump.** Each connection runs a request parser and a
   response parser from `crosswire-h1`. The parsers decide where a message
   ends; they never produce the bytes that are sent. The relay forwards exactly
   the bytes the parser reported as consumed, and keeps the rest pending for the
   next step (a pipelined request, the start of the next response).
2. **Write before read.** The pump reads at most 64 KiB (`READ_BUF`, `pump.rs`),
   writes everything the parser consumed to the other side, flushes, and only
   then reads again. A slow reader on one side therefore slows the writer on the
   other through TCP, and the proxy never holds more than one read buffer of
   in-flight bytes per direction.
3. **The request body and the upstream read run concurrently.** `send_request`
   (`relay.rs`) writes the rewritten head, then `select!`s between the request
   body pump and a reader that waits for the response head. Interim responses
   are forwarded as they arrive, so `Expect: 100-continue` works; a final
   response that arrives while the body is still uploading (a 401 or 413) is
   relayed, the upload stops, the capture is marked cut and the connection
   closes after that response.
4. **Only the head is ever rewritten, and every rewrite is a recorded diff.**
   `Proxy-Connection` and `Proxy-Authorization` lines are deleted. On the plain
   path an absolute-form target becomes origin-form and a missing `Host` is
   inserted. Each change is a `Diff` on the recorded request, with a span into
   the original head. Everything else, including the `Connection` header and
   the body framing, passes untouched.
5. **A capture window beside the pump.** The bytes the relay forwards are pushed
   into a `CaptureBuf` (`capture.rs`) that counts and SHA-256-hashes every byte
   and keeps a prefix. Three limits shape it:
   - `capture_cap` (50 MiB) is the most bytes of one body kept. A longer body
     records `truncated_at` and keeps the first `capture_cap` bytes.
   - The buffer starts empty and grows geometrically: 4 KiB, then doubling,
     never past the cap.
   - `body_budget` (256 MiB) bounds every capture on one listener together. The
     budget is charged for the buffer's **capacity**, before the buffer grows.
     When a growth would exceed it, the capture is dropped for the rest of that
     body: `capture_dropped` is set and only the length and hash survive.
   The relay never waits on the store or on the budget. Capture is a copy into
   an owned buffer plus a hash update, and a refused capture only costs the
   stored bytes.
6. **The stored body is the raw framed wire bytes.** Chunk sizes, extensions and
   trailers are stored as they were forwarded. The model's `Body::Inline`
   contract is "as on the wire", and a decoded view is derived by re-parsing the
   stored bytes behind the stored head. `relay`, the length and SHA-256 of every
   byte that crossed, is recorded only when the stored bytes are not the whole
   body: truncated at the cap, dropped for the budget, or cut short by a close or
   timeout.
7. **Submit without blocking the relay.** A finished exchange is built by the
   `Recorder` (`record.rs`) and queued with `try_send` on a bounded channel
   (1,024 records) to one dedicated recorder thread, which calls the store's
   `ExchangeSink` in order. The sink may block, because a spill to the journal
   fsyncs on the submitting thread (ADR-0002), so it runs on that thread and
   not on tokio's blocking pool, which DNS lookups share: a slow store cannot
   hold up new connections. The capture's budget reservation travels with the
   record and is released once the sink has accepted or refused the exchange.
   When the queue is full the exchange is dropped, counted in `sink_full`,
   logged at most once every 10 s, and marked with a `Gap` marker for its id;
   a refused exchange (`Full`) is counted and logged the same way. The relay
   carries on either way. `Proxy::run` returns only after the recorder thread
   has handed everything queued to the sink.
8. **Generated responses are recorded as `Fidelity::Generated`.** When the proxy
   writes a response itself (a 502 for a failed connect, a 504 for a timeout, a
   400 for an unparseable request), it builds the bytes and the recorded
   `Message` from the same function (`generated.rs`), marks the message
   `Generated`, and records the exchange in the `Error` state when a request was
   read. A reader can tell what an upstream said from what Crosswire said.
9. **No `Session` on the proxy path.** `crosswire-client` supplies the
   destination gate, DNS, TCP and TLS (`Transport::connect`) and nothing else.
   The proxy reads and writes the upstream connection itself, so the same pump
   code serves both sides.
10. **Failures are explicit.** Before the response head, a failure becomes a
    generated 502 or 504 and a recorded `Error` exchange. After the head, the
    client has already been told something, so a failure closes the connection
    and records an `Error` exchange with a cut body, usually with a marker. A message that
    cannot be delimited turns the connection into an opaque tunnel (recorded as
    `Tunnel` with reason `Undelimitable`) instead of guessing. The full table is
    in the [`crosswire-proxy` README](../../crates/crosswire-proxy/README.md).
11. **Redaction stays on the way out.** The proxy records what it received.
    Redaction applies when data leaves (exports, logs, copy-as), as spec
    section 11.4 says, not at rest.

## Alternatives considered

**A hyper-based proxy.** Rejected. hyper parses a message into a typed header
map and writes it back out: header case, order, duplicate headers, obsolete
folding, bare line feeds and ambiguous framing are normalised or refused, which
is the opposite of the fidelity promise. Making it relay raw bytes would mean
fighting the library at every step. The proxy's own test of the idea is the
corpus golden: 208 runs of byte-exact relay that cover all 26 diagnostic codes.

**`Session::send` from `crosswire-client` for the upstream leg.** Rejected.
`Session` takes a whole `WritePlan` and writes it before it reads a byte, so it
buffers the entire request in memory, cannot start reading the response while a
body is still uploading, and cannot honour `Expect: 100-continue` (its README
says so). A proxy needs the opposite: bytes flowing as they arrive.

**A separate pack-writer stream (spec section 7.1).** Deferred. The spec
describes streaming large bodies to a blob writer through a bounded window,
beside the exchange. In alpha.1 the capture is an in-memory buffer and the body
goes into the store inline, through the same queue as the exchange: up to 64 MiB
queued, spilled to the journal beyond that, packed by the writer thread. That
is enough for a 50 MiB cap and removes a second durable write path to build and
test next to the first. It does mean a large body costs memory twice over (see
Consequences). A stream into the pack file remains the way to lift the cap.

**Charging the budget for bytes kept instead of buffer capacity.** Rejected
during implementation. Charging the length lets the real allocation run ahead of
the number the budget enforces, so the bound would hold on paper only. Charging
capacity before the allocation makes `capture memory <= body_budget` true by
construction.

## Consequences

- **Memory.** Capture memory never exceeds `body_budget` (256 MiB), because
  every byte of capacity is reserved before it is allocated. Three honest
  qualifications:
  - Utilisation can be low. A 100-byte body holds a 4 KiB buffer, and a buffer
    that has just doubled is half empty, so a body just past a power of two
    charges nearly twice its length.
  - A growth step copies into a bigger buffer, so the old one is briefly alive
    as well, and that old buffer is not charged. At the default cap the largest
    step is from 32 MiB to 50 MiB, so up to 32 MiB per capture that is growing
    at that instant sits outside the budget.
  - After `submit` the bytes belong to the store. They leave the proxy budget
    and only the store's queue (64 MiB and 10,000 exchanges) bounds them, with
    the 1 GiB journal behind it.
- **A big body occupies the queue.** A 50 MiB response is queued whole, taking
  most of the 64 MiB queue; the next exchanges spill to the journal, each
  costing an fsync on a blocking thread. An exchange whose request and response
  bodies together exceed the queue always spills. The journal absorbs bursts;
  when it is full, recording stops with a gap marker and the proxy keeps
  relaying.
- **Truncation is visible.** A cut-off body records `truncated_at`, a dropped one
  `capture_dropped`, and both keep the full length and hash in `relay`.
  `resp_len` in the store is the relay length when it is present.
- **Intercept is additive.** `Decision` has `Forward` and `Drop` today and is
  `#[non_exhaustive]`. Holding a message for editing (`Hold`, `Modify`) arrives
  with intercept in alpha.2 as new variants. The handler methods are
  synchronous and called inline today, so a hold will need the relay to await a
  decision; that is design work for alpha.2, not something this ADR settles.
- **The relay is serial per connection.** One exchange is in flight at a time
  per client connection, and pipelined requests are queued in the pump up to
  `max_pipelined` (16), beyond which the connection is closed with a
  `LimitExceeded` marker.
- **Known gaps.** Auto-passthrough after repeated handshake rejections is off
  (it needs scope, alpha.2); rejections are counted and one `Error` marker is
  written per window. `inline_memory` is a field of `ProxyLimits` but is not
  consulted yet. `total` (300 s) bounds a whole exchange, so a download longer
  than that is cut with a `timeout: total` marker, and so is any long-lived
  response that is still HTTP: a server-sent event stream or a long poll is
  cut at 300 s the same way (a WebSocket is a tunnel and only has the tunnel
  idle limit).
