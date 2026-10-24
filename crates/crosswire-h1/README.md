# crosswire-h1

Sans-IO, byte-exact HTTP/1 for security testing tools. Part of
[Crosswire](https://github.com/kalash33/crosswire/blob/main/README.md), an open-source Rust web security testing platform.

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
   `Progress::Done` the parser returns `Done` (consuming nothing) until
   `reset()`; the unconsumed tail of `input` is the start of the next
   pipelined message. For the next response on a keep-alive connection call
   `reset_response(ctx)` with the next request's context, and use `is_idle()`
   to tell a clean close between messages from a truncated one.
2. **Ownership.** The parser owns one bounded head buffer
   (`ParseLimits::head_size`). It becomes `RawMessage::bytes()`, and every span in
   `HeadView`, `HeaderSpan` and `StartLine` is an offset into it. The spans in
   `Progress::BodyData`, `ChunkExt` and `Trailers` are offsets into the `input`
   slice of the `feed` call that produced them; keep those bytes if you need
   them. `body_diagnostics()` spans are absolute offsets from the first body
   byte. The head is buffered by the parser; body-phase tokens (the chunk-size line with its extension, and the
   trailer block) are never buffered across calls. When one is incomplete,
   `feed` returns `NeedMore` with `consumed` at the token start, and the caller
   must **re-present** the unconsumed bytes together with the new data.
   Events other than `NeedMore` may also report `consumed == 0`. In particular
   `ChunkExt` is a peek: its span points into the current input, the chunk-size
   line is not consumed, and the caller re-presents the same bytes to receive
   that chunk's `BodyData`. `Trailers` can return with its bytes consumed up to
   the terminator, and `Done` arrives on the next call. Looping until
   `NeedMore` (or `Done`) is therefore the correct caller shape; do not assume
   that every event advances the input. Re-presented bytes must be identical
   to the ones presented before; the parser does not verify them.
3. **Obtaining a message.** `Progress::Head(RawMessage)` is the only way to get
   a `RawMessage`. It holds the frozen head bytes plus an optional inline body
   attached with `with_inline_body`. The inline body is the raw framed body
   bytes as on the wire (chunk-size lines and trailers included), so
   `write_exact` reproduces the message byte for byte.
4. **Lenient contract.** Under `Profile::Lenient`, input that contains a
   start-line terminator and an empty line within `head_size` never produces a
   `ParseError`; every other irregularity is a `Diagnostic`. The fatal cases are:
   a limit exceeded (`HeadTooLarge`, `LineTooLong`, `TooManyHeaders`,
   `TrailerTooLarge`, `TooManyChunks`), no terminator within `head_size`
   (reported as `HeadTooLarge`), end of input before the message is complete
   (`UnexpectedEof`), a chunk size that is not hexadecimal, and a malformed
   line break after chunk data (both `InvalidChunkSize`). `Profile::Strict`
   does not make more input fatal; it raises the severity of diagnostics.

See [ADR-0001](https://github.com/kalash33/crosswire/blob/main/docs/adr/0001-h1-sans-io-contract.md) for the reasoning
behind rule 2.

## Example

The reference feed loop, as a proxy would run it: bytes arrive in reads that
cut through the head, a chunk-size line and the trailer block. Unconsumed
bytes are kept and re-presented with the next read. This block is compiled and
run as a doctest.

```rust
use crosswire_h1::{Parser, ParserConfig, Progress};

let reads: [&[u8]; 3] = [
    b"POST /up HTTP/1.1\r\nHost: example.com\r\nTransfer-",
    b"Encoding: chunked\r\n\r\n5;sig=ab\r\nhel",
    b"lo\r\n0\r\nX-Sum: 1\r\n\r\n",
];
let mut parser = Parser::request(&ParserConfig::default());
let (mut carry, mut body, mut done) = (Vec::new(), Vec::new(), false);
for read in reads {
    carry.extend_from_slice(read); // unconsumed bytes lead, new bytes follow
    let mut off = 0;
    loop {
        let input = &carry[off..];
        let step = parser.feed(input).expect("fatal parse error");
        match &step.event {
            Progress::Head(msg) => println!("framing {:?}", msg.head().framing()),
            // A peek: nothing is consumed, the same bytes are fed again.
            Progress::ChunkExt(ext) => println!("ext {:?}", &input[ext.range()]),
            Progress::BodyData { data, .. } => body.extend_from_slice(&input[data.range()]),
            Progress::Trailers(t) => println!("{} trailer field(s)", t.len()),
            Progress::Done => done = true,
            _ => {}
        }
        off += step.consumed;
        if matches!(step.event, Progress::NeedMore | Progress::Done) {
            break;
        }
    }
    carry.drain(..off); // keep only what must be re-presented
    if done {
        break; // `carry` now starts the next pipelined message
    }
}
if !done {
    parser.feed_eof().expect("truncated message"); // ends an until-close body
}
assert_eq!(body, b"hello");
```

## Editing

`apply_edits(&msg, &edits, &cfg, ctx, reason)` is the one way to change a
message. Each `Edit { span, replacement }` addresses `head ++ body` (an edit
past the head needs an inline body); edits are sorted by start and must not
overlap. The new bytes are parsed again with `cfg` (and, for a response, the
`ResponseContext` of the request it answers; `None` means an ordinary
request), so the edited `RawMessage` comes from `Progress::Head` like any
other and its spans index its own head. The result, `Edited`, carries the
message, one `Change` per edit with its span in the **new** bytes, and the
diagnostics the edit introduced. Refusals are `EditError::{Overlap,
OutOfRange, BodyNotInline, Reparse}`.

Two helpers build common edits: `content_length_edit` sets `Content-Length`
to the inline body's length (inserting the header if it is missing, and
returning `None` when `Transfer-Encoding` is present or nothing changes), and
`crlf_edits` turns bare LF line endings in the head into CRLF.

```rust
use bytes::Bytes;
use crosswire_core::ChangeReason;
use crosswire_h1::{apply_edits, content_length_edit, Edit, Parser, ParserConfig, Progress};

let cfg = ParserConfig::default();
let raw = b"POST /api HTTP/1.1\r\nHost: example.com\r\nContent-Length: 2\r\n\r\n";
let Progress::Head(msg) = Parser::request(&cfg).feed(raw).unwrap().event else { panic!() };
let msg = msg.with_inline_body(Bytes::from_static(b"{}"));

// Rewrite the body, then fix the length in a second pass.
let body_at = msg.bytes().len() as u32;
let body = Edit {
    span: crosswire_core::Span::new(body_at, body_at + 2),
    replacement: Bytes::from_static(b"{\"admin\":true}"),
};
let step = apply_edits(&msg, &[body], &cfg, None, ChangeReason::InterceptEdit).unwrap();
let fix = content_length_edit(&step.message).unwrap();
let done = apply_edits(&step.message, &[fix], &cfg, None, ChangeReason::ContentLength).unwrap();
let out = &done.message;
let len = out.head().header(out.bytes(), "content-length").unwrap();
assert_eq!(&out.bytes()[len.range()], b"14");
assert_eq!(out.body().unwrap().as_ref(), b"{\"admin\":true}");
```

## Features

| feature | default | effect |
| --- | --- | --- |
| `std` | yes | enables `std` in `crosswire-core` and `bytes`; the crate is `no_std` + `alloc` without it |
| `http` | no | implies `std`; `TryFrom<&RawMessage>` for `http::Request<()>` and `http::Response<()>` (lossy, with `HttpConvertError` carrying spans) |
| `serde` | no | `Serialize`/`Deserialize` for the plain-data types |

`just features` builds with `--no-default-features` and with each feature alone.

## Diagnostics and corpus

- [Diagnostic catalogue](https://github.com/kalash33/crosswire/blob/main/docs/h1-diagnostics.md): all 26 codes, with
  severities under each profile.
- [Fidelity corpus](https://github.com/kalash33/crosswire/blob/main/corpus/h1/README.md): odd-but-real HTTP/1 messages,
  each parsed, diagnosed and written back byte-identical.
