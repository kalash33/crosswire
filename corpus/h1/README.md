# crosswire-h1 golden fidelity corpus

Every odd-but-real HTTP/1 shape the library knows about: parsed, diagnosed and
written back byte-identical. The harness is
`crates/crosswire-h1/tests/corpus.rs` (`corpus_golden`); the coverage checker is
`scripts/corpus-check.py`.

## Format

Each case lives in `cases/` as a pair, named `<code-or-case>-<n>`:

- `<name>.msg` is the raw bytes, exactly as they would appear on the wire
  (binary-safe; `.gitattributes` marks `cases/**` as `binary` so git never
  normalises line endings). A case may hold one message, or a message followed by
  a second pipelined one.
- `<name>.expect.json` holds the expectations:

```json
{
  "kind": "request",
  "ctx": { "method": "GET", "is_connect": false },
  "framing": "ambiguous",
  "resolved": "chunked",
  "rule": "transfer_encoding_wins",
  "diagnostics": ["conflicting_framing"],
  "pipelined": false,
  "body": "abc"
}
```

| field | meaning |
| --- | --- |
| `kind` | `request` or `response` |
| `ctx` | responses only: `method` is the method of the request being answered, `is_connect` whether it was a CONNECT |
| `framing` | `none`, `content_length`, `chunked`, `until_close`, `upgrade` or `ambiguous`; omitted only for an `error` case that fails before the head |
| `resolved`, `rule` | only when `framing` is `ambiguous`: the resolved variant and the `ResolutionRule` in snake_case |
| `diagnostics` | head diagnostics plus body diagnostics, by code name, sorted, duplicates removed |
| `pipelined` | `true` when a second message follows the first; the harness checks the tail starts a new message |
| `body` / `body_hex` | the expected decoded payload (UTF-8 string, or hex when it is not UTF-8); omit when there is no body |
| `error` | a fatal `ParseError` instead of a clean parse, in snake_case: `head_too_large`, `line_too_long`, `too_many_headers`, `unexpected_eof`, `invalid_chunk_size`, `trailer_too_large` or `too_many_chunks` |
| `limits` | optional overrides of `ParseLimits` for this case, e.g. `{ "head_size": 64 }`; keys are the `ParseLimits` field names |
| `known_bug` | a description of a parser bug this case exposes; the harness skips and names the case |

`invalid_chunk_size` is a fatal parse error rather than a message diagnostic, so
cases for it carry `"error": "invalid_chunk_size"` and the checker counts that
`error` value toward code coverage. The limit fixtures (`head_too_large-1`,
`line_too_long-1`/`-2`, `too_many_headers-1`, `trailer_too_large-1`,
`too_many_chunks-1`) use `limits` to make a small message hit each limit.

A case too large to commit is described by `<name>.gen.json` instead
(`kind`, `head`, `body_byte`, `body_len`, `framing`, `diagnostics`); the
harness builds `head` followed by `body_len` copies of `body_byte` at test time.

## What the harness asserts

Every case is fed through the canonical feed loop (`drive` in
`crates/crosswire-h1/tests/common/mod.rs`, also used by the split test). For
every case: the error (if any), the framing variant (and `resolved`/`rule`), the
diagnostic list, `write_exact(head)` equals the head bytes,
`write_exact(head.with_inline_body(raw body bytes))` equals the consumed input,
the raw body spans tile the body, that all the input is consumed (or, when
`pipelined`, that a second message follows), and the decoded body. An `error`
case may stop before the head; then only the error and an empty diagnostic list
are checked.

## Adding a case

1. Write the raw bytes to `cases/<name>.msg` (use `printf` or a script, never an
   editor that rewrites line endings).
2. Write `cases/<name>.expect.json`.
3. Run `cargo test -p crosswire-h1 --test corpus` and
   `python3 -I scripts/corpus-check.py corpus/h1`.

If the parser disagrees with what the spec requires, record the case with the
spec behaviour and add `"known_bug"`; do not bend the expectation to the bug.

## Contributor rules

- **A case must round-trip byte-for-byte.** The corpus is the proof of fidelity;
  a case that cannot be written back identical is a bug in the library, not in
  the case.
- Cases must not depend on the operating system's line endings.
- Every one of the 26 diagnostic codes must appear in at least one case.

## Licence

Cases here are written for Crosswire and share the repository licence. A case
derived from the llhttp test suite (MIT licence, Copyright Fedor Indutny and
contributors) must say so in its `expect.json` through a `"source"` note and
keep the llhttp licence notice available in this directory.
