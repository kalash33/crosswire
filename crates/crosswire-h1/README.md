# crosswire-h1

Sans-IO, byte-exact HTTP/1 for security testing tools. Part of
[Crosswire](https://github.com/crosswire-rs/crosswire/blob/main/README.md), an open-source Rust web security testing platform.

## Charter

- Parse tolerantly, with diagnostics. Malformed or ambiguous input is accepted
  and annotated, not normalised away or refused.
- Write byte-exact. `write_exact` emits the bytes that were received.
- Expose spans for every element: start line parts, header names, values and
  whole lines, chunk extensions, trailers and body data.
- Sans-IO. The crate never reads or writes a socket; you feed it bytes.

Non-goal: a general-purpose HTTP client.

## Contract

1. **Feeding.** `Parser::feed(&mut self, input: &[u8]) -> Result<Step, ParseError>`
   returns `Step { consumed, event }`. Loop until `Progress::NeedMore`. After
   `Progress::Done`, the unconsumed tail of `input` is the start of the next
   pipelined message; hand it to a fresh parser or call `reset()`.
2. **Ownership.** The parser owns one bounded head buffer
   (`ParseLimits::head_size`). It becomes `RawMessage::bytes()`, and every span in
   `HeadView`, `HeaderSpan` and `StartLine` is an offset into it. The spans in
   `Progress::BodyData`, `ChunkExt` and `Trailers` are offsets into the `input`
   slice of the `feed` call that produced them; keep those bytes if you need
   them. Body-phase tokens (the chunk-size line with its extension, and the
   trailer block) are never buffered across calls. When one is incomplete,
   `feed` returns `NeedMore` with `consumed` at the token start, and the caller
   must **re-present** the unconsumed bytes together with the new data.
   Events other than `NeedMore` may also report `consumed == 0`. In particular
   `ChunkExt` is a peek: its span points into the current input, the chunk-size
   line is not consumed, and the caller re-presents the same bytes to receive
   that chunk's `BodyData`. `Trailers` can return with its bytes consumed up to
   the terminator, and `Done` arrives on the next call. Looping until
   `NeedMore` (or `Done`) is therefore the correct caller shape; do not assume
   that every event advances the input.
3. **Obtaining a message.** `Progress::Head(RawMessage)` is the only way to get
   a `RawMessage`. It holds the frozen head bytes plus an optional inline body
   attached with `with_inline_body`.
4. **Lenient contract.** Under `Profile::Lenient`, input that contains a
   start-line terminator and an empty line within `head_size` never produces a
   `ParseError`; every other irregularity is a `Diagnostic`. The fatal cases are:
   a limit exceeded, no terminator within `head_size`, end of input before the
   empty line, and a chunk size that is not hexadecimal. `Profile::Strict` does
   not make more input fatal; it raises the severity of diagnostics.

See [ADR-0001](https://github.com/crosswire-rs/crosswire/blob/main/docs/adr/0001-h1-sans-io-contract.md) for the reasoning
behind rule 2.

## Example

Parse a request with an obsolete line fold, print the diagnostics, and write it
back unchanged. This block is compiled and run as a doctest.

```rust
use bytes::BytesMut;
use crosswire_h1::{write_exact, Parser, ParserConfig, Progress};

let input = b"GET /a HTTP/1.1\r\nHost: example.com\r\nX-Note: one\r\n  two\r\n\r\n";
let mut parser = Parser::request(&ParserConfig::default());
let step = parser.feed(input).unwrap();
let Progress::Head(msg) = step.event else { panic!("expected a head") };
for d in msg.diagnostics() {
    println!("{} {:?} at {}..{}", d.code().as_str(), d.severity(), d.span().start(), d.span().end());
}
let mut out = BytesMut::new();
write_exact(&msg, &mut out);
assert_eq!(&out[..], &input[..step.consumed]);
assert!(msg.diagnostics().iter().any(|d| d.code().as_str() == "obs_fold"));
```

## Features

| feature | default | effect |
| --- | --- | --- |
| `std` | yes | enables `std` in `crosswire-core` and `bytes`; the crate is `no_std` + `alloc` without it |
| `http` | no | implies `std`; `TryFrom<&RawMessage>` for `http::Request<()>` and `http::Response<()>` (lossy, with `HttpConvertError` carrying spans) |
| `serde` | no | `Serialize`/`Deserialize` for the plain-data types |

`just features` builds with `--no-default-features` and with each feature alone.

## Diagnostics and corpus

- [Diagnostic catalogue](https://github.com/crosswire-rs/crosswire/blob/main/docs/h1-diagnostics.md): all 25 codes, with
  severities under each profile.
- [Fidelity corpus](https://github.com/crosswire-rs/crosswire/blob/main/corpus/h1/README.md): odd-but-real HTTP/1 messages,
  each parsed, diagnosed and written back byte-identical.
