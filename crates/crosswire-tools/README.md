# crosswire-tools

Client-side tools built on `crosswire-client`: the **repeater** and the
recording recipe it shares.

## Repeater

[`Repeater`] takes a raw HTTP/1.1 request, checks it against a scope, sends it,
follows redirects and records every exchange. It never depends on the store: it
writes through an `ExchangeSink` and reports security-relevant actions through
an audit hook.

```text
SendRequest { tab, source, host, port, tls, sni, verify, raw, mode, follow, reuse, allow_out_of_scope }
        │
        ├─ scope check  (connection host/SNI  vs  message Host header) ─► ScopeState
        │     out of scope + (strict || !allow_out_of_scope)  ─►  SendError::OutOfScope
        │
        ├─ Mode::Raw       send `raw` untouched
        ├─ Mode::Assisted  crlf_edits, then content_length_edit  (recorded as diffs)
        │                  Transfer-Encoding present ─► "content-length: skipped, TE present"
        │
        ├─ record_exchange  ─►  namespace = Repeater(tab),  tag = source:<ulid>
        │
        └─ follow 3xx Location  (Off | InScope | Always, ≤ 10 hops)  ─►  SendOutcome
```

### Modes

- **Raw** sends the bytes exactly as given, framing the response from a
  best-effort parse of the request method (falling back to until-close).
- **Assisted** turns bare-LF line endings into CRLF and inserts or corrects
  `Content-Length`, recording each edit as a `Diff`. When the request carries
  `Transfer-Encoding`, the `Content-Length` edit is skipped and the skip is
  reported in `SendOutcome::skipped`. The two edits are applied as two
  sequential `apply_edits` passes (line endings first, then `Content-Length`),
  because one combined pass would collide at the blank-line offset. The first
  pass's `Change` spans are in the bytes after that pass, the second's in the
  final bytes, so undoing them means reversing the second pass first (last in,
  first out). Both passes are recorded with the `ContentLength` change reason.

### Scope

The request is evaluated twice against the same compiled scope — once for the
connection identity (the host or SNI being dialled) and once for the message
identity (the `Host` header) — and the `PairVerdict` maps to a
`ScopeState` (`In`, `Out`, `Empty`, `HostMismatch`). Under a **non-empty**
scope an out-of-scope request (`Out` or `HostMismatch`) is refused with
`SendError::OutOfScope` when strict mode is on or `allow_out_of_scope` is
false; otherwise it is sent and audited. An **empty** scope never refuses and
never prompts.

### Audit hook

The hook is called `(action, target, result)`. A send that succeeds writes
`repeater.send` with the dialled `host:port` and the mode as the target. A send
that goes ahead out of scope (`allow_out_of_scope` with strict mode off) writes
`repeater.send.out_of_scope` first. For that row the target is the dialled
`host:port`, while `SendError::OutOfScope` carries the `Host` header value when a
send is refused; the two can differ (that difference is the host mismatch).

### Redirects

On a `3xx` response with a `Location` header, the repeater follows the chain up
to ten hops. `InScope` stops as soon as the target leaves scope; `Always`
follows regardless; `Off` never follows. Each hop is its own recorded
exchange owned by the same tab, and relative `Location` values are resolved
against the current URL. Every hop is sent as a plain `GET` with
`Connection: close`, whatever the redirect status: a `307` or `308` does **not**
preserve the original method or body.

## Recording recipe

[`record::record_exchange`] sends a request over a `Session`, reads the whole
response, and submits an `Exchange` built from the exact bytes written and read
to an `ExchangeSink`. It files the exchange in a namespace, records a scope
verdict, attaches request diffs, and links an optional source exchange as a
`source:<ulid>` tag. This recipe originated in `crosswire-recorder-tests`,
which now re-exports a four-argument wrapper for its existing tests.
