# ADR-0001: the crosswire-h1 sans-IO contract

Status: Accepted

## Context

`crosswire-h1` is the wire layer for a tool that must see and reproduce exactly
what is on the wire, including malformed and ambiguous messages. It cannot own
sockets (callers are proxies, repeaters, tests and fuzzers with different I/O),
and it cannot normalise input. It needs a contract that is small enough to state,
that holds under arbitrary input fragmentation, and that keeps every span
pointing at real bytes.

## Decision

Four rules (spec section 5.1):

1. **Feeding.** `Parser::feed(&[u8]) -> Result<Step, ParseError>` returns
   `Step { consumed, event }`. The caller loops until `NeedMore`. After `Done`
   the unconsumed tail is the next pipelined message (fresh parser or `reset()`).
2. **Ownership.** The parser owns one bounded head buffer (`head_size`); it
   becomes `RawMessage::bytes()` and all head spans index into it. Body-phase
   spans (`BodyData`, `ChunkExt`, `Trailers`) index into the `input` of the
   `feed` call that produced them. Body-phase tokens (chunk-size line with
   extension, trailer block) are never buffered across calls.
3. **Obtaining a message.** `Progress::Head(RawMessage)` is the only source of a
   `RawMessage`; an optional inline body is attached by the caller.
4. **Lenient contract.** Under `Profile::Lenient`, input with a start-line
   terminator and an empty line within `head_size` never yields `ParseError`;
   everything else is a `Diagnostic`. Fatal cases: limit exceeded, no terminator
   within `head_size`, EOF before the empty line, non-hex chunk size.

### Body-phase tokens: re-present, do not buffer (Task 10b)

When a chunk-size line or a trailer block is split across `feed` calls, the
parser returns `NeedMore` with `consumed` at the start of the token and keeps no
bytes of it. The caller presents the unconsumed bytes again, followed by the new
data. Chunk data itself is consumed incrementally and is not subject to this.

Events other than `NeedMore` may also report `consumed == 0`. `ChunkExt` is a
peek: its span points into the current input, the chunk-size line is not
consumed, and the caller re-presents the same bytes to receive the chunk's
`BodyData`. `Trailers` can return with its bytes consumed up to the terminator,
and `Done` arrives on the next call. "Loop until `NeedMore`" is the correct
caller shape; a caller must not assume every event advances the input.

**Rejected alternative: internal buffering with clamped spans.** The first
implementation copied incomplete tokens into a decoder-owned buffer. A span
reported for a token completed on a later call would then point partly into
earlier input that the caller's current `input` slice no longer contains, so the
decoder clamped such spans to the current slice. Clamping made `ChunkExt` and
`Trailers` spans wrong (truncated) whenever input was split inside those
tokens. The split-at-every-byte property test found this in 5 corpus cases. It
also meant a second, hidden buffer with its own size accounting. Re-presenting
removes the buffer and the clamp: a span always refers to the `input` of the
call that returned it, and results do not depend on how the input was split.

## Consequences

- Spans are never clamped or approximated; every span is valid for the buffer
  its rule names. This is checked by `tests/split_prop.rs` (whole input vs every
  two-way split vs byte-by-byte) and the fuzz targets.
- Callers must keep unconsumed bytes and re-present them. A caller that drops
  them loses data. The rule is stated on `Parser::feed` and `Progress::NeedMore`.
- Feeding a long incomplete token byte-by-byte rescans it each time; the size
  line and trailer block are bounded by `max_chunk_size_line` and
  `max_trailer_size`, which bounds the rescan cost.
- Memory is bounded without a body buffer: the only owned buffer is the head,
  limited by `head_size`.
- `Strict` changes severities, not fatality, so rule 4 is about `Lenient` and
  strictness never adds a failure mode that callers have to handle.
