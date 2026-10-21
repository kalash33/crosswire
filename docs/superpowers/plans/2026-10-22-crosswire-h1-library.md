# Crosswire alpha.1, part 1: `crosswire-core` + `crosswire-h1` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A sans-IO, byte-exact HTTP/1 library (`crosswire-h1`) on shared types (`crosswire-core`), with a golden fidelity corpus, a split-at-every-byte property test, a fuzz target and a benchmark, so the recorder in part 2 can parse through it.

**Architecture:** A Cargo workspace. `crosswire-core` holds `Span`, `Diagnostic`, `DiagnosticCode`, `Profile`, `Change` and depends on `bytes` only. `crosswire-h1` holds a streaming `Parser` state machine that owns one bounded head buffer, emits `Step { consumed, event }`, resolves body framing itself, and never panics; a `write_exact` writer; a `BodyEncoder`. Tests are corpus-driven: each corpus file carries its expected diagnostics and framing.

**Tech Stack:** Rust stable (MSRV stable-2 for binaries; the published crates carry `rust-version` = stable-6), `bytes`, `proptest`, `cargo-fuzz`, `criterion`, `serde`/`serde_json` (dev + optional feature), `http` (optional feature), `just`.

**Spec:** `docs/superpowers/specs/2026-10-22-crosswire-design.md` (v4), §4.7 (principles), §5 crate table, §5.1 (the h1 contract, the four rules, the 25-code catalogue), §6.0 framing resolution, §11.6 head limits, §13 golden suite and split proptest, §2.3 corpus.

## Global Constraints

- Published crates: `#![forbid(unsafe_code)]`, `#![deny(missing_docs)]`, every public enum `#[non_exhaustive]`, private fields with accessors (§4.7, §5.1).
- `crosswire-core` depends on `bytes` only; `alloc` required; `std` feature gates only `std::error::Error` / io impls (§5).
- `crosswire-h1` depends on `crosswire-core` and `bytes` only; features `std` (default), `http`, `serde` (§5.1). CI builds each crate with `--no-default-features` and with each feature alone.
- Default `ParseLimits`: `head_size` 64 KiB, `max_headers` 256, `max_line` 16 KiB, `max_chunk_size_line` 4 KiB, `max_trailer_size` 16 KiB (§11.6 head row).
- Lenient contract: input containing a start-line terminator and an empty line within `head_size` never yields `ParseError`; only a limit exceeded, no terminator within `head_size`, EOF before the empty line, or a non-numeric chunk size are fatal (§5.1 rule 4).
- `Span { start: u32, end: u32 }`, exclusive, `Copy`; head spans index the parser-owned head buffer; body spans index the `input` of the `feed` call that produced them (§5.1 rules 2–3).
- Licence: `Apache-2.0 OR MIT`. Commit messages end with the attribution lines from the session reminder.
- Hours are logged in `docs/log.md` per task (§14 velocity check).

## Review Focus

1. A request whose head arrives one byte per `feed` call, including a split inside the `\r\n\r\n` terminator, must produce identical spans and diagnostics to the same bytes fed at once (Task 10 split proptest; Task 3 `needmore_across_terminator` test).
2. A head that is exactly `head_size` bytes with the terminator ending at byte `head_size` must parse; one byte more must return `ParseError::HeadTooLarge` and never allocate past the limit (Task 3 `head_size_boundary` tests).
3. `Content-Length: 0` with a trailing `\r\n\r\n` followed immediately by a second request in the same `feed` must report `consumed` at the end of the first message and leave the second untouched (Task 7 `pipelined_two_requests` test).
4. A chunked body whose chunk-size line is `0` followed by trailers and then `\r\n` must emit `Trailers` then `Done`, and `write_exact` of the recorded bytes must be identical (Task 7 `chunked_trailers` and Task 8 round-trip).
5. A response to `HEAD`, or with status 204/304/1xx, carrying `Content-Length: 10` must frame as `BodyFraming::None` with no `BodyData` event (Task 5 `response_context_no_body` tests).

---

### Task 0: Workspace scaffold

**Files:**
- Create: `Cargo.toml` (workspace), `rust-toolchain.toml`, `.gitignore`, `justfile`, `deny.toml`, `LICENSE-APACHE`, `LICENSE-MIT`, `README.md`, `docs/log.md`, `.github/workflows/ci.yml`
- Create: `crates/crosswire-core/Cargo.toml`, `crates/crosswire-core/src/lib.rs` (empty with crate attrs)
- Create: `crates/crosswire-h1/Cargo.toml`, `crates/crosswire-h1/src/lib.rs` (empty with crate attrs)

**Interfaces:**
- Produces: workspace members `crosswire-core`, `crosswire-h1`; `just test`, `just lint`, `just fuzz-smoke` recipes.

- [ ] **Step 1: Write the workspace `Cargo.toml`** with `[workspace] members = ["crates/*"]`, `resolver = "2"`, and `[workspace.package]` `edition = "2021"`, `license = "Apache-2.0 OR MIT"`, `rust-version = "1.86"`, `repository`, `[workspace.lints.rust] unsafe_code = "forbid", missing_docs = "deny"`. Each crate `Cargo.toml` sets `lints.workspace = true`, `publish = false` for now, and for `crosswire-h1` `[dependencies] crosswire-core = { path = "../crosswire-core" }, bytes = "1"`, `[features] default = ["std"], std = ["crosswire-core/std"], http = ["dep:http"], serde = ["dep:serde", "crosswire-core/serde"]`, `[dev-dependencies] proptest = "1", serde_json = "1"`.
- [ ] **Step 2: Write `rust-toolchain.toml`** pinning `channel = "stable"`, and `justfile` with recipes `test` (`cargo test --workspace --all-features`), `lint` (`cargo clippy --workspace --all-targets --all-features -- -D warnings` and `cargo fmt --check`), `features` (for each crate: `cargo check -p <crate> --no-default-features`, then `--no-default-features --features <each>`), `fuzz-smoke` (placeholder that prints "no targets yet"; replaced in Task 11).
- [ ] **Step 3: Write `.github/workflows/ci.yml`** running `just lint`, `just test`, `just features` on `ubuntu-latest` and `macos-latest`, with actions pinned by SHA (`actions/checkout`, `dtolnay/rust-toolchain`), plus a `cargo-deny` step.
- [ ] **Step 4: Verify** `cargo build --workspace` and `just lint` pass with the empty crates.
- [ ] **Step 5: Add `docs/log.md`** with a table header `| date | task | hours | notes |` and a first row for this task.
- [ ] **Step 6: Commit** `chore: scaffold workspace with crosswire-core and crosswire-h1`.

### Task 1: `crosswire-core` types

**Files:**
- Create: `crates/crosswire-core/src/span.rs`, `src/diagnostic.rs`, `src/profile.rs`, `src/change.rs`, `src/hint.rs`; modify `src/lib.rs` to re-export.
- Test: `crates/crosswire-core/tests/span.rs`, `tests/diagnostic_codes.rs`

**Interfaces:**
- Produces:
  - `pub struct Span` with `pub fn new(start: u32, end: u32) -> Span` (panics only in debug if `end < start`; release clamps), `start()`, `end()`, `len() -> u32`, `is_empty()`, `range() -> core::ops::Range<usize>`; derives `Copy, Clone, Debug, PartialEq, Eq, Hash`.
  - `#[non_exhaustive] pub enum Severity { Info, Warning, Error }`.
  - `#[non_exhaustive] pub enum DiagnosticCode { DuplicateContentLength, ConflictingFraming, ObsFold, BareLf, BareCr, WhitespaceBeforeColon, TrailingSpaceInName, LeadingWhitespaceBeforeStartLine, NonCanonicalHeaderCase, InvalidTokenCharInName, InvalidCharInValue, MissingHost, DuplicateHost, AbsoluteFormTarget, ChunkExtensionPresent, TrailerPresent, ChunkSizeLeadingZeros, InvalidChunkSize, ContentLengthNonNumeric, ContentLengthLeadingPlus, UnknownTransferCoding, BodyAfterNoBodyStatus, Expect100Continue, Http09Request, VersionUnusual }` with `pub const fn as_str(&self) -> &'static str` (snake_case, stable), `pub fn default_severity(&self, profile: Profile) -> Severity` per the §5.1 table, and `pub const ALL: &[DiagnosticCode]` (25 entries).
  - `pub struct Diagnostic { code, span, severity }` with `new(code, span, severity)` and accessors.
  - `#[non_exhaustive] pub enum Profile { Strict, Lenient }` with `impl Default` = `Lenient`.
  - `#[non_exhaustive] pub enum ChangeOp { Insert, Delete, Replace }`; `#[non_exhaustive] pub enum ChangeReason { HopByHop, ContentLength, MatchReplace { rule: String }, IntruderPosition(u16), InterceptEdit, Custom(String) }`; `pub struct Change { op, span, before: bytes::Bytes, after: bytes::Bytes, reason }` with accessors.
  - `#[non_exhaustive] pub enum EncodingHint { Indexed, LiteralWithIndexing, LiteralWithoutIndexing, NeverIndexed }` plus `pub struct FieldHint { encoding: EncodingHint, huffman: bool }` (reserved for the frozen schema; no behaviour).
  - Feature `serde` derives `Serialize, Deserialize` on all of the above; feature `std` implements `std::error::Error` for nothing yet (kept for h1's `ParseError` via a `core::fmt::Display` bound).

- [ ] **Step 1: Write the failing tests**

```rust
// tests/span.rs
use crosswire_core::Span;
#[test] fn span_len_and_range() { let s = Span::new(3, 7); assert_eq!(s.len(), 4); assert!(!s.is_empty()); assert_eq!(s.range(), 3..7); }
#[test] fn empty_span() { assert!(Span::new(5, 5).is_empty()); }
// tests/diagnostic_codes.rs
use crosswire_core::{DiagnosticCode, Profile, Severity};
#[test] fn catalogue_has_25_stable_names() {
    assert_eq!(DiagnosticCode::ALL.len(), 25);
    let names: Vec<&str> = DiagnosticCode::ALL.iter().map(|c| c.as_str()).collect();
    assert_eq!(names[0], "duplicate_content_length");
    assert_eq!(names[24], "version_unusual");
    let mut sorted = names.clone(); sorted.sort(); sorted.dedup(); assert_eq!(sorted.len(), 25);
}
#[test] fn severities_follow_spec_table() {
    assert_eq!(DiagnosticCode::BareLf.default_severity(Profile::Lenient), Severity::Warning);
    assert_eq!(DiagnosticCode::BareLf.default_severity(Profile::Strict), Severity::Error);
    assert_eq!(DiagnosticCode::NonCanonicalHeaderCase.default_severity(Profile::Strict), Severity::Info);
    assert_eq!(DiagnosticCode::InvalidChunkSize.default_severity(Profile::Lenient), Severity::Error);
}
```

- [ ] **Step 2: Run** `cargo test -p crosswire-core` — expected: compile errors (types missing).
- [ ] **Step 3: Implement the types** in the files listed, with the `as_str` table and the severity table copied from spec §5.1 (Lenient/Strict pairs: W/E for the fourteen syntax codes, I/E for `leading_whitespace_before_start_line`, I/I for `non_canonical_header_case`, `absolute_form_target`, `chunk_extension_present`, `trailer_present`, `expect_100_continue`, I/W for `chunk_size_leading_zeros`, E/E for `invalid_chunk_size`).
- [ ] **Step 4: Run** `cargo test -p crosswire-core` and `cargo check -p crosswire-core --no-default-features` — expected: PASS, and no_std build succeeds.
- [ ] **Step 5: Commit** `feat(core): spans, diagnostics catalogue, profile, change types`.

### Task 2: `crosswire-h1` public types and config

**Files:**
- Create: `crates/crosswire-h1/src/limits.rs`, `src/config.rs`, `src/error.rs`, `src/message.rs` (`RawMessage`, `HeadView`, `StartLine`, `HeaderSpan`, `BodyFraming`, `ResolutionRule`, `ResponseContext`), `src/progress.rs` (`Step`, `Progress`); modify `src/lib.rs`.
- Test: `crates/crosswire-h1/tests/config.rs`

**Interfaces:**
- Produces:
  - `#[non_exhaustive] pub struct ParseLimits { pub head_size: u32, pub max_headers: u16, pub max_line: u32, pub max_chunk_size_line: u32, pub max_trailer_size: u32 }` with `impl Default` (spec values) and a builder `ParseLimits::builder()`.
  - `#[non_exhaustive] pub struct ParserConfig { profile: Profile, limits: ParseLimits }` with `ParserConfig::builder().profile(..).limits(..).build()`, `Default` = Lenient + default limits, accessors.
  - `#[non_exhaustive] pub enum ParseError { HeadTooLarge, LineTooLong, TooManyHeaders, NoTerminator, UnexpectedEof, InvalidChunkSize { span: Span }, TrailerTooLarge }` with `Display`; `std::error::Error` under `std`.
  - `pub enum StartLine { Request { method: Span, target: Span, version: Span }, Response { version: Span, status: Span, reason: Span } }` (`#[non_exhaustive]`).
  - `pub struct HeaderSpan` with `name()`, `value()`, `line()` accessors.
  - `#[non_exhaustive] pub enum BodyFraming { None, ContentLength(u64), Chunked, UntilClose, Upgrade, Ambiguous { resolved: Box<BodyFraming>, candidates: Vec<BodyFraming>, rule: ResolutionRule } }`; `#[non_exhaustive] pub enum ResolutionRule { TransferEncodingWins, FirstContentLength, NoBodyStatus, HeadMethod, ConnectSuccess }`.
  - `pub struct HeadView` with `start() -> &StartLine`, `headers() -> &[HeaderSpan]`, `framing() -> &BodyFraming`, `header(&self, buf: &[u8], name: &str) -> Option<Span>` (case-insensitive, first match).
  - `pub struct RawMessage` with `bytes() -> &Bytes` (frozen head), `head() -> &HeadView`, `body() -> Option<&Bytes>`, `diagnostics() -> &[Diagnostic]`, `with_inline_body(self, Bytes) -> Self`, `kind() -> Kind`; `#[non_exhaustive] pub enum Kind { Request, Response }`.
  - `pub struct ResponseContext { pub method: Method, pub is_connect: bool }` with `#[non_exhaustive] pub enum Method { Head, Other }`; `ResponseContext::for_method(bytes: &[u8]) -> ResponseContext`.
  - `pub struct Step { pub consumed: usize, pub event: Progress }`; `#[non_exhaustive] pub enum Progress { NeedMore, Head(RawMessage), BodyData { raw: Span, data: Span }, ChunkExt(Span), Trailers(Vec<HeaderSpan>), Done }`.

- [ ] **Step 1: Write the failing test**

```rust
// tests/config.rs
use crosswire_h1::{ParseLimits, ParserConfig};
use crosswire_core::Profile;
#[test] fn default_limits_match_spec() {
    let l = ParseLimits::default();
    assert_eq!((l.head_size, l.max_headers, l.max_line, l.max_chunk_size_line, l.max_trailer_size), (65536, 256, 16384, 4096, 16384));
}
#[test] fn builder_overrides() {
    let c = ParserConfig::builder().profile(Profile::Strict).limits(ParseLimits::builder().head_size(1024).build()).build();
    assert_eq!(c.profile(), Profile::Strict); assert_eq!(c.limits().head_size, 1024);
}
```

- [ ] **Step 2: Run** `cargo test -p crosswire-h1 --test config` — expected: compile error.
- [ ] **Step 3: Implement** the types above; no parsing logic yet. `HeadView::header` compares ASCII case-insensitively.
- [ ] **Step 4: Run** the test — expected: PASS. Run `cargo doc -p crosswire-h1 --no-deps` — expected: no missing-docs errors.
- [ ] **Step 5: Commit** `feat(h1): public types, limits, config, errors`.

### Task 3: Request head parser (strict happy path, streaming, limits)

**Files:**
- Create: `crates/crosswire-h1/src/parser/mod.rs` (`Parser`, state enum, head buffer), `src/parser/head.rs` (line scanning, header splitting), `src/parser/tests_support.rs` (dev-only helper `feed_all(parser, &[u8]) -> Vec<Step>`).
- Test: `crates/crosswire-h1/tests/head_request.rs`

**Interfaces:**
- Produces: `pub struct Parser`; `Parser::request(cfg: &ParserConfig) -> Parser`; `feed(&mut self, input: &[u8]) -> Result<Step, ParseError>`; `feed_eof(&mut self) -> Result<Step, ParseError>`; `reset(&mut self)`. Internal: `HeadBuf` (a `BytesMut` capped at `head_size`), `State { Head, Body(BodyState), Done }`.
- Consumes: Task 2 types.

- [ ] **Step 1: Write the failing tests**

```rust
// tests/head_request.rs
use crosswire_h1::*;
const REQ: &[u8] = b"GET /a?b=1 HTTP/1.1\r\nHost: x.io\r\nX-A: 1\r\n\r\n";
#[test] fn head_in_one_feed() {
    let mut p = Parser::request(&ParserConfig::default());
    let s = p.feed(REQ).unwrap();
    assert_eq!(s.consumed, REQ.len());
    let Progress::Head(m) = s.event else { panic!() };
    assert_eq!(m.bytes().as_ref(), REQ);
    let StartLine::Request { method, target, version } = m.head().start() else { panic!() };
    assert_eq!(&REQ[method.range()], b"GET"); assert_eq!(&REQ[target.range()], b"/a?b=1"); assert_eq!(&REQ[version.range()], b"HTTP/1.1");
    assert_eq!(m.head().headers().len(), 2);
    assert_eq!(&REQ[m.head().headers()[1].name().range()], b"X-A");
    assert_eq!(&REQ[m.head().headers()[1].value().range()], b"1");
    assert!(m.diagnostics().is_empty());
    assert!(matches!(m.head().framing(), BodyFraming::None));
    assert!(matches!(p.feed(b"").unwrap().event, Progress::Done));
}
#[test] fn needmore_across_terminator() {
    let mut p = Parser::request(&ParserConfig::default());
    let (a, b) = REQ.split_at(REQ.len() - 2);           // split inside "\r\n\r\n"
    let s = p.feed(a).unwrap(); assert_eq!(s.consumed, a.len()); assert!(matches!(s.event, Progress::NeedMore));
    let s = p.feed(b).unwrap(); assert_eq!(s.consumed, 2);
    let Progress::Head(m) = s.event else { panic!() }; assert_eq!(m.bytes().as_ref(), REQ);
}
#[test] fn head_consumed_stops_at_terminator() {
    let mut p = Parser::request(&ParserConfig::default());
    let mut v = REQ.to_vec(); v.extend_from_slice(b"GET / HTTP/1.1\r\n\r\n");
    let s = p.feed(&v).unwrap(); assert_eq!(s.consumed, REQ.len());
}
#[test] fn head_size_boundary() {
    let cfg = ParserConfig::builder().limits(ParseLimits::builder().head_size(64).build()).build();
    let ok = b"GET / HTTP/1.1\r\nHost: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\r\n\r\n"; // exactly 64 bytes
    assert_eq!(ok.len(), 64);
    assert!(matches!(Parser::request(&cfg).feed(ok).unwrap().event, Progress::Head(_)));
    let mut big = ok.to_vec(); big.insert(5, b'/');
    assert!(matches!(Parser::request(&cfg).feed(&big), Err(ParseError::HeadTooLarge)));
}
#[test] fn too_many_headers_and_long_line() {
    let cfg = ParserConfig::builder().limits(ParseLimits::builder().max_headers(1).build()).build();
    assert!(matches!(Parser::request(&cfg).feed(REQ), Err(ParseError::TooManyHeaders)));
    let cfg = ParserConfig::builder().limits(ParseLimits::builder().max_line(8).build()).build();
    assert!(matches!(Parser::request(&cfg).feed(REQ), Err(ParseError::LineTooLong)));
}
#[test] fn eof_before_empty_line() {
    let mut p = Parser::request(&ParserConfig::default());
    p.feed(b"GET / HTTP/1.1\r\nHost: x\r\n").unwrap();
    assert!(matches!(p.feed_eof(), Err(ParseError::UnexpectedEof)));
}
```

- [ ] **Step 2: Run** `cargo test -p crosswire-h1 --test head_request` — expected: compile error (`Parser` missing).
- [ ] **Step 3: Implement the head state machine.** Append `input` to `HeadBuf` up to the first `\r\n\r\n` (or `\n\n`, handled in Task 4), checking `head_size` before copying so no allocation exceeds the limit. On the terminator: scan lines, record `StartLine` and `HeaderSpan`s as offsets into the frozen buffer, apply `max_headers` and `max_line`, freeze with `BytesMut::freeze`, build `RawMessage` with framing `None` for now, move to `State::Body`/`Done`, return `Step { consumed: bytes_taken_from_input, event: Head(msg) }`. `feed` after `Done` returns `Step { consumed: 0, event: Done }`. `feed_eof` in `Head` returns `UnexpectedEof`.
- [ ] **Step 4: Run** the tests — expected: PASS.
- [ ] **Step 5: Commit** `feat(h1): streaming request head parser with limits`.

### Task 4: Lenient diagnostics for the head

**Files:**
- Modify: `crates/crosswire-h1/src/parser/head.rs`; create `src/parser/diag.rs` (push helper that assigns severity by `profile`).
- Test: `crates/crosswire-h1/tests/head_diagnostics.rs`

**Interfaces:**
- Consumes: Task 3 parser. Produces: diagnostics on `RawMessage::diagnostics()` for the 14 head-level codes: `bare_lf`, `bare_cr`, `obs_fold`, `whitespace_before_colon`, `trailing_space_in_name`, `leading_whitespace_before_start_line`, `non_canonical_header_case`, `invalid_token_char_in_name`, `invalid_char_in_value`, `missing_host`, `duplicate_host`, `absolute_form_target`, `http_0_9_request`, `version_unusual`, `expect_100_continue`.

- [ ] **Step 1: Write the failing tests**, one per code, each asserting the code, the span text and the severity under both profiles. Examples:

```rust
fn parse(b: &[u8], profile: Profile) -> RawMessage { /* helper: Parser::request with profile, feed, unwrap Head */ }
fn has(m: &RawMessage, c: DiagnosticCode) -> Option<&Diagnostic> { m.diagnostics().iter().find(|d| d.code() == c) }
#[test] fn bare_lf_terminators() {
    let m = parse(b"GET / HTTP/1.1\nHost: x\n\n", Profile::Lenient);
    let d = has(&m, DiagnosticCode::BareLf).unwrap(); assert_eq!(d.severity(), Severity::Warning);
    assert_eq!(m.head().headers().len(), 1);
}
#[test] fn obs_fold_keeps_bytes() {
    let raw = b"GET / HTTP/1.1\r\nHost: x\r\nX-A: 1\r\n 2\r\n\r\n";
    let m = parse(raw, Profile::Lenient);
    assert!(has(&m, DiagnosticCode::ObsFold).is_some());
    assert_eq!(&raw[m.head().headers()[1].value().range()], b"1\r\n 2");   // value span covers the folded continuation
    assert_eq!(m.bytes().as_ref(), raw);
}
#[test] fn whitespace_before_colon_and_trailing_space() {
    let m = parse(b"GET / HTTP/1.1\r\nHost : x\r\n\r\n", Profile::Strict);
    assert_eq!(has(&m, DiagnosticCode::WhitespaceBeforeColon).unwrap().severity(), Severity::Error);
    assert_eq!(has(&m, DiagnosticCode::TrailingSpaceInName).unwrap().severity(), Severity::Error);
}
#[test] fn missing_and_duplicate_host() {
    assert!(has(&parse(b"GET / HTTP/1.1\r\n\r\n", Profile::Lenient), DiagnosticCode::MissingHost).is_some());
    assert!(has(&parse(b"GET / HTTP/1.1\r\nHost: a\r\nhost: b\r\n\r\n", Profile::Lenient), DiagnosticCode::DuplicateHost).is_some());
}
#[test] fn absolute_form_and_versions() {
    assert!(has(&parse(b"GET http://x/ HTTP/1.1\r\nHost: x\r\n\r\n", Profile::Lenient), DiagnosticCode::AbsoluteFormTarget).is_some());
    assert!(has(&parse(b"GET /\r\n\r\n", Profile::Lenient), DiagnosticCode::Http09Request).is_some());
    assert!(has(&parse(b"GET / HTTP/1.7\r\nHost: x\r\n\r\n", Profile::Lenient), DiagnosticCode::VersionUnusual).is_some());
}
#[test] fn lenient_never_errors_on_terminated_head() {
    for raw in [&b"\r\n\r\nGET / HTTP/1.1\r\nHost: x\r\n\r\n"[..], b"G\x01T / HTTP/1.1\r\nHo\x7fst: x\x01\r\n\r\n", b"GET / HTTP/1.1\r\nX-A: \xff\xfe\r\nHost: x\r\n\r\n"] {
        assert!(Parser::request(&ParserConfig::default()).feed(raw).is_ok(), "{raw:?}");
    }
}
```
(Also: `bare_cr` for a lone `\r` inside a line; `leading_whitespace_before_start_line` for leading CRLF/space skipped and spanned; `non_canonical_header_case` for `host:`; `invalid_token_char_in_name` / `invalid_char_in_value` for control bytes; `expect_100_continue` when `Expect: 100-continue` is present.)

- [ ] **Step 2: Run** `cargo test -p crosswire-h1 --test head_diagnostics` — expected: FAIL (no diagnostics emitted).
- [ ] **Step 3: Implement** line scanning that accepts `\n` and `\r\n` terminators, records obs-fold continuations into the previous header's value span, tolerates every byte in names and values while emitting the matching diagnostic, and detects the HTTP/0.9 and unusual-version shapes. Under `Strict`, the same diagnostics are emitted with `Error` severity; no profile turns a diagnostic into a `ParseError`.
- [ ] **Step 4: Run** the tests — expected: PASS.
- [ ] **Step 5: Commit** `feat(h1): lenient head diagnostics for all head-level codes`.

### Task 5: Body framing resolution

**Files:**
- Create: `crates/crosswire-h1/src/parser/framing.rs` (`resolve(headers, buf, kind, ctx, profile) -> (BodyFraming, Vec<Diagnostic>)`).
- Modify: `src/parser/mod.rs` to call it when the head completes.
- Test: `crates/crosswire-h1/tests/framing.rs`

**Interfaces:**
- Consumes: Task 4 diagnostics plumbing. Produces: `HeadView::framing()` populated; codes `duplicate_content_length`, `content_length_non_numeric`, `content_length_leading_plus`, `conflicting_framing`, `unknown_transfer_coding`, `body_after_no_body_status`. `Parser::response(cfg, ctx)` constructor (status line parsing comes in Task 6; this task adds the constructor and framing rules).

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn content_length_single() { assert!(matches!(framing_of(b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 5\r\n\r\n"), BodyFraming::ContentLength(5))); }
#[test] fn duplicate_equal_content_length_is_diagnostic_not_error() {
    let m = parse(b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 5\r\nContent-Length: 5\r\n\r\n", Profile::Lenient);
    assert!(has(&m, DiagnosticCode::DuplicateContentLength).is_some()); assert!(matches!(m.head().framing(), BodyFraming::ContentLength(5)));
}
#[test] fn conflicting_cl_and_te_resolves_to_chunked_with_rule() {
    let m = parse(b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 5\r\nTransfer-Encoding: chunked\r\n\r\n", Profile::Lenient);
    let BodyFraming::Ambiguous { resolved, candidates, rule } = m.head().framing() else { panic!() };
    assert!(matches!(**resolved, BodyFraming::Chunked)); assert_eq!(candidates.len(), 2); assert_eq!(*rule, ResolutionRule::TransferEncodingWins);
    assert!(has(&m, DiagnosticCode::ConflictingFraming).is_some());
}
#[test] fn differing_content_lengths_resolve_to_first() {
    let m = parse(b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 5\r\nContent-Length: 6\r\n\r\n", Profile::Lenient);
    let BodyFraming::Ambiguous { resolved, rule, .. } = m.head().framing() else { panic!() };
    assert!(matches!(**resolved, BodyFraming::ContentLength(5))); assert_eq!(*rule, ResolutionRule::FirstContentLength);
}
#[test] fn non_numeric_and_leading_plus() {
    let m = parse(b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 5x\r\n\r\n", Profile::Lenient);
    assert!(has(&m, DiagnosticCode::ContentLengthNonNumeric).is_some()); assert!(matches!(m.head().framing(), BodyFraming::None));
    let m = parse(b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: +5\r\n\r\n", Profile::Lenient);
    assert!(has(&m, DiagnosticCode::ContentLengthLeadingPlus).is_some()); assert!(matches!(m.head().framing(), BodyFraming::ContentLength(5)));
}
#[test] fn unknown_transfer_coding_is_until_close_for_responses() {
    let m = parse_response(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: gzip\r\n\r\n", ResponseContext::for_method(b"GET"));
    assert!(has(&m, DiagnosticCode::UnknownTransferCoding).is_some()); assert!(matches!(m.head().framing(), BodyFraming::UntilClose));
}
#[test] fn response_context_no_body() {
    for (raw, ctx) in [
        (&b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\n"[..], ResponseContext::for_method(b"HEAD")),
        (b"HTTP/1.1 204 No Content\r\nContent-Length: 10\r\n\r\n", ResponseContext::for_method(b"GET")),
        (b"HTTP/1.1 304 Not Modified\r\nContent-Length: 10\r\n\r\n", ResponseContext::for_method(b"GET")),
        (b"HTTP/1.1 100 Continue\r\nContent-Length: 10\r\n\r\n", ResponseContext::for_method(b"GET")),
    ] { let m = parse_response(raw, ctx); assert!(matches!(m.head().framing(), BodyFraming::Ambiguous { resolved, .. } if matches!(**resolved, BodyFraming::None))); assert!(has(&m, DiagnosticCode::BodyAfterNoBodyStatus).is_some()); }
}
#[test] fn connect_2xx_and_101_are_upgrade() {
    assert!(matches!(framing_of_response(b"HTTP/1.1 200 OK\r\n\r\n", ResponseContext { method: Method::Other, is_connect: true }), BodyFraming::Upgrade));
    assert!(matches!(framing_of_response(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n\r\n", ResponseContext::for_method(b"GET")), BodyFraming::Upgrade));
}
#[test] fn response_without_length_is_until_close() { assert!(matches!(framing_of_response(b"HTTP/1.1 200 OK\r\n\r\n", ResponseContext::for_method(b"GET")), BodyFraming::UntilClose)); }
```

- [ ] **Step 2: Run** — expected: FAIL.
- [ ] **Step 3: Implement `resolve`** with RFC 9112 precedence: no-body status/method → `None` (as `Ambiguous{rule: NoBodyStatus|HeadMethod}` when a length header is present, plain `None` otherwise); `Upgrade` for 101 and CONNECT 2xx; `Transfer-Encoding` present → `Chunked` if the final coding is `chunked`, else `UntilClose` (responses) or `None` with `unknown_transfer_coding` (requests); both TE and CL → `Ambiguous{TransferEncodingWins}`; multiple CL → equal: diagnostic only; differing: `Ambiguous{FirstContentLength}`; invalid CL → `content_length_non_numeric` and `None` (request) / `UntilClose` (response); request with neither → `None`; response with neither → `UntilClose`.
- [ ] **Step 4: Run** — expected: PASS.
- [ ] **Step 5: Commit** `feat(h1): body framing resolution with ambiguity rules`.

### Task 6: Response status-line parsing

**Files:**
- Modify: `crates/crosswire-h1/src/parser/head.rs` (status-line branch), `src/parser/mod.rs` (`Parser::response`).
- Test: `crates/crosswire-h1/tests/head_response.rs`

**Interfaces:**
- Produces: `Parser::response(cfg: &ParserConfig, ctx: ResponseContext) -> Parser`; `StartLine::Response { version, status, reason }` spans; `RawMessage::kind() == Kind::Response`; `HeadView::status_code(&self, buf) -> Option<u16>`.

- [ ] **Step 1: Write the failing tests**: a normal status line (`HTTP/1.1 404 Not Found` → spans and `status_code() == Some(404)`); an empty reason phrase (`HTTP/1.1 200 \r\n`); a missing reason and space (`HTTP/1.1 200\r\n`) producing `version_unusual`-free parse with empty reason span; a non-numeric status yielding `Progress::Head` with `status_code() == None` and `version_unusual`; `needmore` split inside the status line.
- [ ] **Step 2: Run** — expected: FAIL.
- [ ] **Step 3: Implement** the status-line branch sharing the header scanner with requests; `missing_host`/`duplicate_host` are request-only.
- [ ] **Step 4: Run** — expected: PASS.
- [ ] **Step 5: Commit** `feat(h1): response status-line parsing`.

### Task 7: Body streaming, chunked decoding, pipelining

**Files:**
- Create: `crates/crosswire-h1/src/parser/body.rs` (`BodyState`: `Fixed { remaining }`, `Chunked(ChunkState)`, `UntilClose`), `src/chunked.rs` (`pub struct BodyDecoder` reused by the parser; `BodyEvent { Data { raw, data }, Ext(Span), Trailers(Vec<HeaderSpan>), Done }`).
- Modify: `src/parser/mod.rs`.
- Test: `crates/crosswire-h1/tests/body.rs`

**Interfaces:**
- Produces: `Progress::BodyData { raw, data }` where `raw` covers the bytes of `input` consumed for this event (including chunk framing when chunked) and `data` the payload subset; `Progress::ChunkExt(Span)`; `Progress::Trailers(Vec<HeaderSpan>)` with spans into `input`; `Progress::Done`; `consumed` always ≤ `input.len()`; codes `chunk_extension_present`, `trailer_present`, `chunk_size_leading_zeros`, `invalid_chunk_size` (fatal `ParseError::InvalidChunkSize`). `pub struct BodyDecoder` with `BodyDecoder::chunked(limits) -> Self`, `feed(&mut self, input: &[u8]) -> Result<(usize, BodyEvent), ParseError>`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn fixed_body_across_feeds() {
    let mut p = Parser::request(&ParserConfig::default());
    p.feed(b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 5\r\n\r\n").unwrap();
    let s = p.feed(b"he").unwrap(); assert_eq!(s.consumed, 2); let Progress::BodyData { raw, data } = s.event else { panic!() }; assert_eq!(raw, Span::new(0, 2)); assert_eq!(data, raw);
    let s = p.feed(b"llo!!").unwrap(); assert_eq!(s.consumed, 3);        // stops at the body end, leaves "!!"
    assert!(matches!(p.feed(b"").unwrap().event, Progress::Done));
}
#[test] fn pipelined_two_requests() {
    let two = b"GET /1 HTTP/1.1\r\nHost: x\r\nContent-Length: 0\r\n\r\nGET /2 HTTP/1.1\r\nHost: x\r\n\r\n";
    let mut p = Parser::request(&ParserConfig::default());
    let s = p.feed(two).unwrap(); let first_len = s.consumed; assert!(matches!(s.event, Progress::Head(_)));
    assert!(matches!(p.feed(&two[first_len..]).unwrap(), Step { consumed: 0, event: Progress::Done }));
    p.reset();
    let s = p.feed(&two[first_len..]).unwrap(); assert_eq!(s.consumed, two.len() - first_len);
    let Progress::Head(m) = s.event else { panic!() }; assert_eq!(&m.bytes()[m.head().start_target_range()], b"/2");
}
#[test] fn chunked_with_ext_and_trailers() {
    let body = b"5;x=y\r\nhello\r\n0\r\nX-T: 1\r\n\r\n";
    let mut p = Parser::request(&ParserConfig::default());
    p.feed(b"POST / HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n").unwrap();
    let mut events = vec![]; let mut off = 0;
    loop { let s = p.feed(&body[off..]).unwrap(); off += s.consumed; let done = matches!(s.event, Progress::Done); events.push(s.event); if done { break } }
    assert!(matches!(events[0], Progress::ChunkExt(_)));
    let Progress::BodyData { raw, data } = &events[1] else { panic!() }; assert_eq!(&body[raw.range()], b"5;x=y\r\nhello\r\n"); assert_eq!(&body[data.range()], b"hello");
    assert!(matches!(events[2], Progress::Trailers(ref t) if t.len() == 1));
    assert_eq!(off, body.len());
}
#[test] fn chunk_size_leading_zeros_and_invalid() {
    let mut p = Parser::request(&ParserConfig::default());
    p.feed(b"POST / HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n").unwrap();
    p.feed(b"005\r\nhello\r\n").unwrap();            // leading zeros: diagnostic recorded on the parser's body diagnostics
    assert!(matches!(p.feed(b"zz\r\n"), Err(ParseError::InvalidChunkSize { .. })));
}
#[test] fn until_close_needs_eof() {
    let mut p = Parser::response(&ParserConfig::default(), ResponseContext::for_method(b"GET"));
    p.feed(b"HTTP/1.1 200 OK\r\n\r\n").unwrap();
    assert!(matches!(p.feed(b"abc").unwrap().event, Progress::BodyData { .. }));
    assert!(matches!(p.feed_eof().unwrap().event, Progress::Done));
}
#[test] fn upgrade_is_done_immediately() {
    let mut p = Parser::response(&ParserConfig::default(), ResponseContext::for_method(b"GET"));
    let s = p.feed(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n\r\nrawbytes").unwrap();
    assert_eq!(s.consumed, b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n\r\n".len());
    assert!(matches!(p.feed(b"rawbytes").unwrap(), Step { consumed: 0, event: Progress::Done }));
}
```
(`HeadView::start_target_range()` is a small accessor added here: `Option<Range<usize>>` of the request target.) Body-phase diagnostics are exposed through `Parser::body_diagnostics(&self) -> &[Diagnostic]` since the `RawMessage` was already emitted.

- [ ] **Step 2: Run** — expected: FAIL.
- [ ] **Step 3: Implement** `BodyState` transitions and `BodyDecoder` (chunk-size line bounded by `max_chunk_size_line`, hex parse with the leading-zero diagnostic, extension span, data with partial-chunk support across feeds, the CRLF after data, the `0` chunk, trailers bounded by `max_trailer_size` reusing the header scanner, final CRLF → `Done`). `UntilClose` emits every input as `BodyData` and `Done` on `feed_eof`.
- [ ] **Step 4: Run** — expected: PASS.
- [ ] **Step 5: Commit** `feat(h1): body streaming, chunked decoder, pipelining`.

### Task 8: `write_exact` and `BodyEncoder`

**Files:**
- Create: `crates/crosswire-h1/src/writer.rs` (`pub fn write_exact(msg: &RawMessage, out: &mut BytesMut)`, `pub struct BodyEncoder`).
- Test: `crates/crosswire-h1/tests/writer.rs`

**Interfaces:**
- Produces: `write_exact` copies `msg.bytes()` then `msg.body()` if inline, byte for byte. `BodyEncoder::chunked() -> Self`; `chunk(&mut self, data: &[u8], ext: Option<&[u8]>, out: &mut BytesMut)` writes `{len:x}[;ext]\r\n{data}\r\n`; `trailers(&mut self, t: &[(&[u8], &[u8])], out)`; `finish(&mut self, out)` writes `0\r\n` + trailers + `\r\n` (trailers only if `trailers` was called, else `0\r\n\r\n`).

- [ ] **Step 1: Write the failing tests**: `write_exact` of every message in `tests/head_diagnostics.rs` inputs equals the input (iterate a fixed array of the raw byte strings used there); `BodyEncoder` produces exactly `b"5;x=y\r\nhello\r\n0\r\nX-T: 1\r\n\r\n"` for the Task 7 example; `BodyEncoder::finish` without trailers produces `b"0\r\n\r\n"`.
- [ ] **Step 2: Run** — expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** — expected: PASS.
- [ ] **Step 5: Commit** `feat(h1): write_exact and chunked BodyEncoder`.

### Task 9: Corpus format and seed corpus

**Files:**
- Create: `corpus/h1/README.md` (format, contribution rules, licence note for llhttp-derived cases), `corpus/h1/cases/*.msg` (raw bytes, exact) + `*.expect.json` (`{ "kind": "request"|"response", "ctx": { "method": "GET", "is_connect": false }?, "framing": "...", "diagnostics": ["bare_lf", ...], "body_events": [...] }`), `scripts/corpus-check.py` (asserts every `.msg` has an `.expect.json` and every one of the 25 codes appears at least once).
- Create: `crates/crosswire-h1/tests/corpus.rs` (the golden harness; dev-dependency `serde_json`).

**Interfaces:**
- Produces: `corpus/h1/` with ≥ 60 cases covering all 25 codes plus the §6.0 framing cases (ambiguous CL/TE, differing CLs, no-body statuses, CONNECT, 101, pipelining, 0-length body, > 1 MiB body generated at test time from a `.gen` descriptor rather than committed); the harness `corpus_golden` test that parses each case, compares framing and the sorted diagnostic code list to `expect.json`, and asserts `write_exact` round-trips the bytes.

- [ ] **Step 1: Write the harness test** (`tests/corpus.rs`) that walks `corpus/h1/cases`, loads each pair, runs the parser (request or response per `kind`), collects `Progress` events and body diagnostics, and asserts: framing variant name equals `expect.framing`; sorted `as_str()` codes equal `expect.diagnostics`; `write_exact(head) ++ consumed body bytes == input`; and `consumed` sums to `input.len()` (or to the first message's end when `expect.pipelined == true`).
- [ ] **Step 2: Run** `cargo test -p crosswire-h1 --test corpus` — expected: FAIL (no cases).
- [ ] **Step 3: Write the seed cases**: at least two per code (one minimal, one realistic capture-shaped), the framing cases, and `scripts/corpus-check.py`. Name files `<code-or-case>-<n>.msg`.
- [ ] **Step 4: Run** the harness and `python3 scripts/corpus-check.py corpus/h1` — expected: PASS and "25/25 codes covered, N cases".
- [ ] **Step 5: Commit** `test(h1): golden corpus format, seed cases, harness`.

### Task 10: Split-at-every-byte property test

**Files:**
- Create: `crates/crosswire-h1/tests/split_prop.rs`

**Interfaces:**
- Consumes: the corpus harness loader from Task 9 (move the loader into `tests/common/mod.rs`).

- [ ] **Step 1: Write the test**: for every corpus case, feed the bytes whole and record (framing, diagnostics, every `Progress` event with spans resolved to byte slices, `consumed` total); then for every split point `i in 1..len` feed `[..i]` then `[i..]` (looping on `consumed`), and additionally feed one byte at a time; assert the resolved event sequences and diagnostics are identical. Use `proptest` for a third strategy: random split vectors.
- [ ] **Step 2: Run** `cargo test -p crosswire-h1 --test split_prop --release` — expected: PASS (fix any state-machine bug it finds in Tasks 3–7 first; each fix gets its own commit `fix(h1): ...`).
- [ ] **Step 3: Commit** `test(h1): split-at-every-byte property test`.

### Task 11: Fuzz target, benchmark, CI smoke

**Files:**
- Create: `crates/crosswire-h1/fuzz/Cargo.toml`, `fuzz/fuzz_targets/parse_request.rs`, `fuzz/fuzz_targets/parse_response.rs` (feed in random-sized pieces; assert no panic, `consumed <= input.len()`, and `write_exact` round-trip when `Head` is produced), `crates/crosswire-h1/benches/parse.rs` (criterion: h1 Lenient vs `httparse` on a 300-byte request head; `httparse` as dev-dependency only).
- Modify: `justfile` (`fuzz-smoke`: `cargo +nightly fuzz run parse_request -- -max_total_time=60`), `.github/workflows/ci.yml` (nightly job: fuzz smoke 60 s per target; `bench` job runs `cargo bench --no-run`).

- [ ] **Step 1: Write the fuzz targets and the bench.**
- [ ] **Step 2: Run** `just fuzz-smoke` locally for 60 s per target — expected: no crashes (fix and commit any finding as `fix(h1): ...` with the crashing input added to the corpus).
- [ ] **Step 3: Run** `cargo bench -p crosswire-h1 --bench parse` — expected: completes; record the ratio to `httparse` in `docs/log.md`.
- [ ] **Step 4: Commit** `test(h1): fuzz targets, criterion bench, CI smoke`.

### Task 12: `http` and `serde` features

**Files:**
- Create: `crates/crosswire-h1/src/compat_http.rs` (`#[cfg(feature = "http")] impl TryFrom<&RawMessage> for http::Request<()>` and `for http::Response<()>`, error `HttpConvertError`), `src/serde_impls.rs` (derives for `Step`/`Progress` are not needed; derive on `BodyFraming`, `ResolutionRule`, `StartLine`, `HeaderSpan`).
- Test: `crates/crosswire-h1/tests/compat_http.rs` (gated on the feature).

**Interfaces:**
- Produces: documented lossy conversion: header names lower-cased, obs-fold joined with a single space, duplicate `Host` → error, invalid header bytes → error with the offending span.

- [ ] **Step 1: Write the failing tests**: a clean request converts with method/uri/version/headers matching; an obs-fold value converts to `"1 2"`; a duplicate-host message returns `Err(HttpConvertError::DuplicateHost)`.
- [ ] **Step 2: Run** `cargo test -p crosswire-h1 --features http --test compat_http` — expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the test, then `just features` — expected: PASS for every feature combination.
- [ ] **Step 5: Commit** `feat(h1): optional http and serde features`.

### Task 13: Docs, package dry-run, log

**Files:**
- Create: `crates/crosswire-h1/README.md` (charter from spec §2.1, the four contract rules from §5.1, a 20-line example, the feature matrix), `docs/h1-diagnostics.md` (the 25-code catalogue table generated by a doctest-checked script or written by hand and asserted against `DiagnosticCode::ALL` in a test), `crates/crosswire-core/README.md`, `docs/adr/0001-h1-sans-io-contract.md` (records the four rules and why `Progress::Head(RawMessage)` owns the head buffer).
- Modify: `docs/log.md` (hours per task, measured LoC via `tokei`, LoC/h).

- [ ] **Step 1: Write the docs** and the `docs_catalogue_matches_code` test (reads `docs/h1-diagnostics.md`, asserts every `as_str()` appears exactly once).
- [ ] **Step 2: Run** `cargo doc --workspace --no-deps`, `cargo package -p crosswire-core --allow-dirty --no-verify` then `cargo package -p crosswire-h1 --allow-dirty --no-verify` with `publish = false` temporarily lifted via `--config` or by setting `publish = ["dry-run"]`… use `cargo publish --dry-run -p crosswire-core` and `-p crosswire-h1` instead, in that order — expected: both succeed.
- [ ] **Step 3: Record** in `docs/log.md`: total hours for Tasks 0–13, `tokei` LoC, measured LoC/h, and whether it is below 45 (spec §16 velocity check).
- [ ] **Step 4: Commit** `docs(h1): README, diagnostics catalogue, ADR-0001, velocity log`.

---

## Self-review notes

- **Spec coverage:** §5.1 rules 1–4, all signatures, the 25 codes, `ParseLimits`/`ParserConfig` builders, `write_exact`, `BodyEncoder`, feature matrix, `cargo package` discipline (§4.7), §6.0 framing resolution rules, §13 golden suite + split proptest + fuzz + h1-vs-httparse bench, §2.3 corpus format. **Deferred to a later plan:** `apply_edits`/`Edit`/`Edited`/`EditError` (spec: alpha.2), `MessageBuilder` (h1 0.2), the ≥ 200-capture corpus size (v0.1.0 DoD; this plan seeds ≥ 60).
- **Type consistency:** `Step { consumed, event }`, `Progress::Head(RawMessage)`, `BodyFraming::Ambiguous { resolved, candidates, rule }`, `ResponseContext { method, is_connect }`, `HeadView::framing()`, `Parser::body_diagnostics()` and `HeadView::start_target_range()` are used with the same names in Tasks 3–12.
- **Proportion:** the plan is shorter than the library it describes; code blocks are tests and signatures.
